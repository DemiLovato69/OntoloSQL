use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};

#[derive(Debug, Clone)]
pub struct ExtractCopyOptions {
    pub input: PathBuf,
    pub output_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyTableData {
    pub table_name: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

pub fn extract_copy_blocks(sql: &str) -> Result<Vec<CopyTableData>> {
    let lines = sql.lines().collect::<Vec<_>>();
    let mut index = 0;
    let mut tables = Vec::new();

    while index < lines.len() {
        let line = lines[index].trim_start();

        if !line.starts_with("COPY ") {
            index += 1;
            continue;
        }

        let (table_name, columns) = parse_copy_header(line)?;
        index += 1;
        let mut rows = Vec::new();

        while index < lines.len() {
            let data_line = lines[index];
            if data_line.trim() == "\\." {
                break;
            }

            rows.push(parse_copy_row(data_line));
            index += 1;
        }

        if index == lines.len() {
            bail!(
                "COPY block for table '{}' does not terminate with '\\.'",
                table_name
            );
        }

        tables.push(CopyTableData {
            table_name,
            columns,
            rows,
        });

        index += 1;
    }

    Ok(tables)
}

pub fn extract_copy_to_csv_files(options: ExtractCopyOptions) -> Result<Vec<PathBuf>> {
    let sql = fs::read_to_string(&options.input)?;
    let tables = extract_copy_blocks(&sql)?;

    fs::create_dir_all(&options.output_dir)?;

    let mut outputs = Vec::with_capacity(tables.len());
    for table in tables {
        let output_path = options.output_dir.join(format!("{}.csv", table.table_name));
        write_copy_table_to_csv(&table, &output_path)?;
        outputs.push(output_path);
    }

    Ok(outputs)
}

fn write_copy_table_to_csv(table: &CopyTableData, output_path: &Path) -> Result<()> {
    let mut writer = csv::Writer::from_path(output_path)?;
    writer.write_record(&table.columns)?;

    for row in &table.rows {
        if row.len() != table.columns.len() {
            bail!(
                "table '{}' has a row with {} values but {} columns",
                table.table_name,
                row.len(),
                table.columns.len()
            );
        }
        writer.write_record(row)?;
    }

    writer.flush()?;
    Ok(())
}

fn parse_copy_header(line: &str) -> Result<(String, Vec<String>)> {
    let line = line.trim();
    let open_paren = line
        .find('(')
        .ok_or_else(|| anyhow!("invalid COPY header: missing '('"))?;
    let close_paren = line[open_paren + 1..]
        .find(')')
        .map(|offset| open_paren + 1 + offset)
        .ok_or_else(|| anyhow!("invalid COPY header: missing ')'"))?;
    let from_stdin = line[close_paren + 1..].trim();

    if !from_stdin.eq_ignore_ascii_case("FROM stdin;") {
        bail!("unsupported COPY header '{}'", line);
    }

    let table_part = line["COPY ".len()..open_paren].trim();
    let table_name = parse_copy_object_name(table_part);
    let columns = line[open_paren + 1..close_paren]
        .split(',')
        .map(|column| parse_copy_identifier(column.trim()))
        .collect::<Vec<_>>();

    if columns.is_empty() {
        bail!(
            "COPY block for table '{}' does not declare any columns",
            table_name
        );
    }

    Ok((table_name, columns))
}

fn parse_copy_object_name(value: &str) -> String {
    value
        .split('.')
        .last()
        .map(parse_copy_identifier)
        .unwrap_or_default()
}

fn parse_copy_identifier(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        trimmed[1..trimmed.len() - 1].replace("\"\"", "\"")
    } else {
        trimmed.to_owned()
    }
}

fn parse_copy_row(line: &str) -> Vec<String> {
    line.split('\t').map(decode_copy_field).collect()
}

fn decode_copy_field(field: &str) -> String {
    if field == "\\N" {
        return String::new();
    }

    let mut decoded = String::with_capacity(field.len());
    let mut chars = field.chars();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            decoded.push(ch);
            continue;
        }

        match chars.next() {
            Some('b') => decoded.push('\u{0008}'),
            Some('f') => decoded.push('\u{000C}'),
            Some('n') => decoded.push('\n'),
            Some('r') => decoded.push('\r'),
            Some('t') => decoded.push('\t'),
            Some('v') => decoded.push('\u{000B}'),
            Some('\\') => decoded.push('\\'),
            Some(other) => decoded.push(other),
            None => decoded.push('\\'),
        }
    }

    decoded
}

#[cfg(test)]
mod tests {
    use super::{decode_copy_field, extract_copy_blocks};

    #[test]
    fn extracts_copy_block_into_rows() {
        let sql = r#"CREATE TABLE public.netflix_shows (
    show_id text NOT NULL,
    title text
);

COPY public.netflix_shows (show_id, title) FROM stdin;
s1	Dick Johnson Is Dead
s2	Blood & Water
\.
"#;

        let tables = extract_copy_blocks(sql).expect("copy extraction should succeed");
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].table_name, "netflix_shows");
        assert_eq!(tables[0].columns, vec!["show_id", "title"]);
        assert_eq!(tables[0].rows.len(), 2);
        assert_eq!(tables[0].rows[0], vec!["s1", "Dick Johnson Is Dead"]);
    }

    #[test]
    fn decodes_postgres_copy_escapes_and_nulls() {
        assert_eq!(decode_copy_field("\\N"), "");
        assert_eq!(decode_copy_field("line\\nwrap"), "line\nwrap");
        assert_eq!(decode_copy_field("tab\\tvalue"), "tab\tvalue");
        assert_eq!(decode_copy_field("slash\\\\value"), "slash\\value");
    }
}
