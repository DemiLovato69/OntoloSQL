use anyhow::{Result, bail};
use sqlparser::ast::{AlterTableOperation, ColumnOption, ObjectName, Statement, TableConstraint};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

use crate::schema::{
    Column, DatabaseSchema, ForeignKey, SqlEnumType, SqlRoutine, SqlRoutineArg, Table,
};

pub fn parse_postgres_schema(sql: &str) -> Result<DatabaseSchema> {
    let dialect = PostgreSqlDialect {};
    let mut tables = Vec::new();
    let mut foreign_keys = Vec::new();
    let mut routines = Vec::new();
    let mut enum_types = Vec::new();

    for statement_sql in split_sql_statements(sql) {
        let parsed = match Parser::parse_sql(&dialect, &statement_sql) {
            Ok(statements) => statements,
            Err(_) => {
                if let Some(enum_type) = parse_create_enum_type(&statement_sql) {
                    enum_types.push(enum_type);
                }
                continue;
            }
        };

        for statement in parsed {
            match statement {
                Statement::CreateTable {
                    name,
                    columns,
                    constraints,
                    ..
                } => {
                    let table_name = parse_object_name(&name);
                    let mut parsed_columns = Vec::with_capacity(columns.len());
                    let mut primary_key = Vec::new();

                    for column in columns {
                        let column_name = column.name.value.clone();
                        let mut nullable = true;
                        let mut default = None;

                        for option in column.options {
                            match option.option {
                                ColumnOption::NotNull => nullable = false,
                                ColumnOption::Null => nullable = true,
                                ColumnOption::Default(expr) => default = Some(expr.to_string()),
                                ColumnOption::Unique { is_primary, .. } if is_primary => {
                                    primary_key.push(column_name.clone());
                                    nullable = false;
                                }
                                ColumnOption::ForeignKey {
                                    foreign_table,
                                    referred_columns,
                                    ..
                                } => {
                                    foreign_keys.push(ForeignKey {
                                        constraint_name: option.name.map(|name| name.value),
                                        source_table: table_name.clone(),
                                        source_columns: vec![column_name.clone()],
                                        target_table: parse_object_name(&foreign_table),
                                        target_columns: referred_columns
                                            .into_iter()
                                            .map(|column| column.value)
                                            .collect(),
                                    });
                                }
                                _ => {}
                            }
                        }

                        parsed_columns.push(Column {
                            name: column_name,
                            sql_type: column.data_type.to_string(),
                            nullable,
                            default,
                        });
                    }

                    for constraint in constraints {
                        match constraint {
                            TableConstraint::PrimaryKey { columns, .. } => {
                                primary_key.extend(columns.into_iter().map(|column| column.value));
                            }
                            TableConstraint::ForeignKey {
                                name,
                                columns,
                                foreign_table,
                                referred_columns,
                                ..
                            } => {
                                foreign_keys.push(ForeignKey {
                                    constraint_name: name.map(|name| name.value),
                                    source_table: table_name.clone(),
                                    source_columns: columns
                                        .into_iter()
                                        .map(|column| column.value)
                                        .collect(),
                                    target_table: parse_object_name(&foreign_table),
                                    target_columns: referred_columns
                                        .into_iter()
                                        .map(|column| column.value)
                                        .collect(),
                                });
                            }
                            _ => {}
                        }
                    }

                    if parsed_columns.is_empty() {
                        bail!("table '{}' does not define any columns", table_name);
                    }

                    tables.push(Table {
                        name: table_name,
                        columns: parsed_columns,
                        primary_key,
                    });
                }
                Statement::AlterTable {
                    name, operations, ..
                } => {
                    let table_name = parse_object_name(&name);

                    for operation in operations {
                        match operation {
                            AlterTableOperation::AddConstraint(TableConstraint::PrimaryKey {
                                columns,
                                ..
                            }) => {
                                if let Some(table) =
                                    tables.iter_mut().find(|table| table.name == table_name)
                                {
                                    table.primary_key =
                                        columns.into_iter().map(|column| column.value).collect();
                                }
                            }
                            AlterTableOperation::AddConstraint(TableConstraint::ForeignKey {
                                name,
                                columns,
                                foreign_table,
                                referred_columns,
                                ..
                            }) => {
                                foreign_keys.push(ForeignKey {
                                    constraint_name: name.map(|name| name.value),
                                    source_table: table_name.clone(),
                                    source_columns: columns
                                        .into_iter()
                                        .map(|column| column.value)
                                        .collect(),
                                    target_table: parse_object_name(&foreign_table),
                                    target_columns: referred_columns
                                        .into_iter()
                                        .map(|column| column.value)
                                        .collect(),
                                });
                            }
                            _ => {}
                        }
                    }
                }
                Statement::CreateFunction {
                    name,
                    args,
                    return_type,
                    ..
                } => {
                    routines.push(SqlRoutine {
                        name: parse_object_name(&name),
                        args: args
                            .unwrap_or_default()
                            .into_iter()
                            .filter_map(|arg| {
                                arg.name.map(|name| SqlRoutineArg {
                                    name: name.value,
                                    sql_type: arg.data_type.to_string(),
                                    has_default: arg.default_expr.is_some(),
                                })
                            })
                            .collect(),
                        return_type: return_type.map(|data_type| data_type.to_string()),
                    });
                }
                _ => {}
            }
        }
    }

    Ok(DatabaseSchema {
        tables,
        foreign_keys,
        routines,
        enum_types,
    })
}

fn parse_object_name(name: &ObjectName) -> String {
    name.0
        .last()
        .map(|identifier| identifier.value.clone())
        .unwrap_or_default()
}

fn parse_create_enum_type(sql: &str) -> Option<SqlEnumType> {
    let mut parser = EnumTypeParser::new(sql);
    parser.parse()
}

struct EnumTypeParser<'a> {
    sql: &'a str,
    offset: usize,
}

impl<'a> EnumTypeParser<'a> {
    fn new(sql: &'a str) -> Self {
        Self { sql, offset: 0 }
    }

    fn parse(&mut self) -> Option<SqlEnumType> {
        self.consume_keyword("create")?;
        self.consume_keyword("type")?;
        let name = self.parse_identifier()?;
        self.consume_keyword("as")?;
        self.consume_keyword("enum")?;
        self.consume_char('(')?;

        let mut values = Vec::new();
        loop {
            values.push(self.parse_string_literal()?);
            self.skip_whitespace();
            if self.consume_char(',').is_some() {
                continue;
            }
            self.consume_char(')')?;
            break;
        }

        Some(SqlEnumType { name, values })
    }

    fn consume_keyword(&mut self, keyword: &str) -> Option<()> {
        self.skip_whitespace();
        let end = self.offset.checked_add(keyword.len())?;
        let candidate = self.sql.get(self.offset..end)?;
        if !candidate.eq_ignore_ascii_case(keyword) {
            return None;
        }
        if self
            .sql
            .get(end..)
            .and_then(|rest| rest.chars().next())
            .is_some_and(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        {
            return None;
        }
        self.offset = end;
        Some(())
    }

    fn consume_char(&mut self, expected: char) -> Option<()> {
        self.skip_whitespace();
        let rest = self.sql.get(self.offset..)?;
        let ch = rest.chars().next()?;
        if ch != expected {
            return None;
        }
        self.offset += ch.len_utf8();
        Some(())
    }

    fn parse_identifier(&mut self) -> Option<String> {
        self.skip_whitespace();
        let rest = self.sql.get(self.offset..)?;
        let mut chars = rest.char_indices();
        let (_, first) = chars.next()?;

        if first == '"' {
            let mut output = String::new();
            while let Some((index, ch)) = chars.next() {
                if ch == '"' {
                    if chars
                        .clone()
                        .next()
                        .is_some_and(|(_, next_ch)| next_ch == '"')
                    {
                        output.push('"');
                        chars.next();
                        continue;
                    }
                    self.offset += index + ch.len_utf8();
                    return Some(output);
                }
                output.push(ch);
            }
            return None;
        }

        if !(first == '_' || first.is_ascii_alphabetic()) {
            return None;
        }

        let mut end = first.len_utf8();
        for (index, ch) in chars {
            if ch == '_' || ch.is_ascii_alphanumeric() {
                end = index + ch.len_utf8();
            } else {
                break;
            }
        }
        let identifier = rest.get(..end)?.to_owned();
        self.offset += end;
        Some(identifier)
    }

    fn parse_string_literal(&mut self) -> Option<String> {
        self.skip_whitespace();
        let rest = self.sql.get(self.offset..)?;
        let mut chars = rest.char_indices();
        let (_, first) = chars.next()?;
        if first != '\'' {
            return None;
        }

        let mut output = String::new();
        while let Some((index, ch)) = chars.next() {
            if ch == '\'' {
                if chars
                    .clone()
                    .next()
                    .is_some_and(|(_, next_ch)| next_ch == '\'')
                {
                    output.push('\'');
                    chars.next();
                    continue;
                }
                self.offset += index + ch.len_utf8();
                return Some(output);
            }
            output.push(ch);
        }
        None
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self
            .sql
            .get(self.offset..)
            .and_then(|rest| rest.chars().next())
        {
            if !ch.is_whitespace() {
                break;
            }
            self.offset += ch.len_utf8();
        }
    }
}

fn split_sql_statements(sql: &str) -> Vec<String> {
    let sql = strip_copy_payload(sql);
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut chars = sql.chars().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut line_comment = false;
    let mut block_comment = false;
    let mut dollar_quote_tag: Option<String> = None;

    while let Some(ch) = chars.next() {
        if line_comment {
            current.push(ch);
            if ch == '\n' {
                line_comment = false;
            }
            continue;
        }

        if block_comment {
            current.push(ch);
            if ch == '*' && chars.peek() == Some(&'/') {
                current.push(chars.next().expect("peeked slash must exist"));
                block_comment = false;
            }
            continue;
        }

        if let Some(tag) = &dollar_quote_tag {
            current.push(ch);

            if ch == '$' && current.ends_with(tag) {
                dollar_quote_tag = None;
            }
            continue;
        }

        if in_single_quote {
            current.push(ch);
            if ch == '\'' {
                if chars.peek() == Some(&'\'') {
                    current.push(chars.next().expect("peeked quote must exist"));
                } else {
                    in_single_quote = false;
                }
            }
            continue;
        }

        if in_double_quote {
            current.push(ch);
            if ch == '"' {
                in_double_quote = false;
            }
            continue;
        }

        match ch {
            '\'' => {
                in_single_quote = true;
                current.push(ch);
            }
            '"' => {
                in_double_quote = true;
                current.push(ch);
            }
            '-' if chars.peek() == Some(&'-') => {
                current.push(ch);
                current.push(chars.next().expect("peeked dash must exist"));
                line_comment = true;
            }
            '/' if chars.peek() == Some(&'*') => {
                current.push(ch);
                chars.next();
                current.push('*');
                block_comment = true;
            }
            '$' => {
                let mut probe = String::from("$");
                let lookahead = chars.clone();

                for next in lookahead {
                    probe.push(next);
                    if next == '$' {
                        break;
                    }
                    if !(next == '_' || next.is_ascii_alphanumeric()) {
                        probe.clear();
                        break;
                    }
                }

                if probe.len() >= 2 && probe.ends_with('$') {
                    current.push_str(&probe);
                    for _ in 1..probe.len() {
                        chars.next();
                    }
                    dollar_quote_tag = Some(probe);
                } else {
                    current.push(ch);
                }
            }
            ';' => {
                if !current.trim().is_empty() {
                    statements.push(current.trim().to_owned());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    if !current.trim().is_empty() {
        statements.push(current.trim().to_owned());
    }

    statements
}

fn strip_copy_payload(sql: &str) -> String {
    let mut filtered = String::new();
    let mut in_copy_data = false;

    for line in sql.lines() {
        if in_copy_data {
            if line.trim() == "\\." {
                in_copy_data = false;
            }
            continue;
        }

        filtered.push_str(line);
        filtered.push('\n');

        if line.trim_start().starts_with("COPY ") && line.trim_end().ends_with("FROM stdin;") {
            in_copy_data = true;
        }
    }

    filtered
}

#[cfg(test)]
mod tests {
    use super::{parse_postgres_schema, split_sql_statements};

    #[test]
    fn parses_table_level_primary_key() {
        let schema = parse_postgres_schema(
            r#"
            CREATE TABLE employee (
                id UUID NOT NULL,
                employee_number VARCHAR(32) NOT NULL,
                PRIMARY KEY (id)
            );
            "#,
        )
        .expect("schema should parse");

        assert_eq!(schema.tables.len(), 1);
        assert_eq!(schema.tables[0].name, "employee");
        assert_eq!(schema.tables[0].primary_key, vec!["id"]);
        assert_eq!(schema.tables[0].columns[1].sql_type, "VARCHAR(32)");
        assert!(schema.foreign_keys.is_empty());
    }

    #[test]
    fn parses_column_level_primary_key() {
        let schema = parse_postgres_schema(
            r#"
            CREATE TABLE project (
                id BIGINT PRIMARY KEY,
                name TEXT
            );
            "#,
        )
        .expect("schema should parse");

        assert_eq!(schema.tables[0].primary_key, vec!["id"]);
        assert!(!schema.tables[0].columns[0].nullable);
    }

    #[test]
    fn parses_alter_table_foreign_key_and_unquotes_names() {
        let schema = parse_postgres_schema(
            r#"
            CREATE TABLE IF NOT EXISTS "software_part_numbers" (
                "id" serial NOT NULL,
                "make_id" integer NOT NULL,
                PRIMARY KEY ("id")
            );
            CREATE TABLE IF NOT EXISTS "make" (
                "id" serial NOT NULL,
                PRIMARY KEY ("id")
            );
            ALTER TABLE "software_part_numbers"
                ADD CONSTRAINT "software_part_numbers_fk5"
                FOREIGN KEY ("make_id") REFERENCES "make"("id");
            "#,
        )
        .expect("schema should parse");

        assert_eq!(schema.tables[0].name, "software_part_numbers");
        assert_eq!(schema.foreign_keys.len(), 1);
        assert_eq!(schema.foreign_keys[0].source_table, "software_part_numbers");
        assert_eq!(schema.foreign_keys[0].source_columns, vec!["make_id"]);
        assert_eq!(schema.foreign_keys[0].target_table, "make");
        assert_eq!(schema.foreign_keys[0].target_columns, vec!["id"]);
    }

    #[test]
    fn parses_alter_table_primary_key() {
        let schema = parse_postgres_schema(
            r#"
            CREATE TABLE public.lego_sets (
                set_num character varying(255) NOT NULL,
                name character varying(255) NOT NULL
            );

            ALTER TABLE ONLY public.lego_sets
                ADD CONSTRAINT lego_sets_pkey PRIMARY KEY (set_num);
            "#,
        )
        .expect("schema should parse");

        assert_eq!(schema.tables.len(), 1);
        assert_eq!(schema.tables[0].name, "lego_sets");
        assert_eq!(schema.tables[0].primary_key, vec!["set_num"]);
    }

    #[test]
    fn ignores_unsupported_statements_around_valid_tables() {
        let schema = parse_postgres_schema(
            r#"
            CREATE EXTENSION IF NOT EXISTS "pgcrypto";
            CREATE TYPE deal_stage AS ENUM ('Lead', 'Closed Won');
            CREATE TABLE users (
                user_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                email VARCHAR(255) UNIQUE NOT NULL
            );
            CREATE OR REPLACE FUNCTION update_modified_column()
            RETURNS TRIGGER AS $$
            BEGIN
                NEW.updated_at = now();
                RETURN NEW;
            END;
            $$ language 'plpgsql';
            CREATE TRIGGER update_users_modtime
                BEFORE UPDATE ON users
                FOR EACH ROW EXECUTE FUNCTION update_modified_column();
            "#,
        )
        .expect("schema should parse");

        assert_eq!(schema.tables.len(), 1);
        assert_eq!(schema.tables[0].name, "users");
    }

    #[test]
    fn splits_dollar_quoted_functions_without_breaking_on_inner_semicolons() {
        let statements = split_sql_statements(
            r#"
            CREATE OR REPLACE FUNCTION update_modified_column()
            RETURNS TRIGGER AS $$
            BEGIN
                NEW.updated_at = now();
                RETURN NEW;
            END;
            $$ language 'plpgsql';
            CREATE TABLE users (
                user_id UUID PRIMARY KEY
            );
            "#,
        );

        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("RETURNS TRIGGER"));
        assert!(statements[1].starts_with("CREATE TABLE users"));
    }

    #[test]
    fn skips_copy_payload_and_keeps_following_alter_table_statements() {
        let statements = split_sql_statements(
            r#"COPY public.lego_sets (set_num, name) FROM stdin;
abc-1	Test Set
\.
ALTER TABLE ONLY public.lego_sets
    ADD CONSTRAINT lego_sets_pkey PRIMARY KEY (set_num);
"#,
        );

        assert_eq!(statements.len(), 2);
        assert!(statements[0].starts_with("COPY public.lego_sets"));
        assert!(statements[1].contains("ADD CONSTRAINT lego_sets_pkey PRIMARY KEY"));
    }

    #[test]
    fn parses_create_function_arguments() {
        let schema = parse_postgres_schema(
            r#"
            CREATE TABLE asset (
                asset_id varchar(20) PRIMARY KEY,
                vin varchar(17) NOT NULL
            );

            CREATE OR REPLACE FUNCTION create_asset(
                p_asset_id varchar(20),
                p_vin varchar(17),
                p_metadata jsonb DEFAULT NULL
            )
            RETURNS asset
            LANGUAGE sql
            AS $$
                SELECT NULL::asset;
            $$;
            "#,
        )
        .expect("schema should parse");

        assert_eq!(schema.routines.len(), 1);
        assert_eq!(schema.routines[0].name, "create_asset");
        assert_eq!(schema.routines[0].args.len(), 3);
        assert_eq!(schema.routines[0].args[0].name, "p_asset_id");
        assert_eq!(schema.routines[0].args[2].sql_type, "JSONB");
        assert!(schema.routines[0].args[2].has_default);
        assert_eq!(schema.routines[0].return_type.as_deref(), Some("asset"));
    }
}
