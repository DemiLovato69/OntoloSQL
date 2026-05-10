use std::collections::HashMap;

use anyhow::{Result, anyhow, bail};
use convert_case::{Case, Casing};

use crate::ontology::{
    LinkDefinition, LinkEndpointDefinition, ModuleDefinition, ObjectDefinition, PropertyDefinition,
};
use crate::schema::{Column, DatabaseSchema, ForeignKey, Table};

pub fn map_schema_to_ontology(schema: &DatabaseSchema) -> Result<ModuleDefinition> {
    let mut objects = Vec::with_capacity(schema.tables.len());
    let mut entity_names_by_table = HashMap::with_capacity(schema.tables.len());
    let mut warnings = Vec::new();
    let mut incoming_foreign_key_targets: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut outgoing_foreign_key_sources: HashMap<&str, Vec<&str>> = HashMap::new();

    for foreign_key in &schema.foreign_keys {
        incoming_foreign_key_targets
            .entry(foreign_key.target_table.as_str())
            .or_default()
            .extend(foreign_key.target_columns.iter().map(String::as_str));
        outgoing_foreign_key_sources
            .entry(foreign_key.source_table.as_str())
            .or_default()
            .extend(foreign_key.source_columns.iter().map(String::as_str));
    }

    for table in &schema.tables {
        let entity_names = entity_names(&table.name);
        entity_names_by_table.insert(table.name.clone(), entity_names.clone());
        let primary_key = infer_primary_key_column(
            table,
            incoming_foreign_key_targets
                .get(table.name.as_str())
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            outgoing_foreign_key_sources
                .get(table.name.as_str())
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        )?;

        if let Some(warning) = &primary_key.warning {
            warnings.push(format!("table '{}': {}", table.name, warning));
        }

        objects.push(map_table(table, &entity_names, &primary_key)?);
    }

    let links = schema
        .foreign_keys
        .iter()
        .map(|foreign_key| map_foreign_key(foreign_key, &entity_names_by_table))
        .collect::<Result<Vec<_>>>()?;

    Ok(ModuleDefinition {
        objects,
        links,
        warnings,
    })
}

fn map_table(
    table: &Table,
    entity_names: &EntityNames,
    primary_key: &PrimaryKeySelection<'_>,
) -> Result<ObjectDefinition> {
    let mut properties: Vec<PropertyDefinition> = table
        .columns
        .iter()
        .map(map_column)
        .collect::<Result<Vec<_>>>()?;

    if let Some(property) = &primary_key.synthetic_property {
        properties.push(property.clone());
    }

    let primary_key_property = primary_key.property_api_name.clone();
    let title_property_api_name = choose_title_property(&properties, &primary_key_property)
        .ok_or_else(|| {
            anyhow!(
                "table '{}' has no columns available to use as a title property",
                table.name
            )
        })?;

    Ok(ObjectDefinition {
        const_name: entity_names.object_api_name.clone(),
        api_name: entity_names.object_api_name.clone(),
        display_name: entity_names.display_name.clone(),
        plural_display_name: entity_names.plural_display_name.clone(),
        title_property_api_name,
        primary_key_property_api_name: primary_key_property,
        properties,
    })
}

fn map_column(column: &Column) -> Result<PropertyDefinition> {
    let api_name = to_property_api_name(&column.name);
    let display_name = column.name.to_case(Case::Title);
    let osdk_type = map_sql_type(&column.sql_type).to_owned();

    Ok(PropertyDefinition {
        api_name,
        display_name,
        osdk_type,
    })
}

fn map_foreign_key(
    foreign_key: &ForeignKey,
    entity_names_by_table: &HashMap<String, EntityNames>,
) -> Result<LinkDefinition> {
    let source = entity_names_by_table
        .get(&foreign_key.source_table)
        .ok_or_else(|| anyhow!("unknown source table '{}'", foreign_key.source_table))?;
    let target = entity_names_by_table
        .get(&foreign_key.target_table)
        .ok_or_else(|| anyhow!("unknown target table '{}'", foreign_key.target_table))?;

    let source_column = match foreign_key.source_columns.as_slice() {
        [name] => name,
        _ => bail!(
            "foreign key from '{}' to '{}' uses multiple source columns, which is not supported in the MVP",
            foreign_key.source_table,
            foreign_key.target_table
        ),
    };

    if foreign_key.target_columns.len() > 1 {
        bail!(
            "foreign key from '{}' to '{}' uses multiple target columns, which is not supported in the MVP",
            foreign_key.source_table,
            foreign_key.target_table
        );
    }

    let api_name = format!(
        "{}To{}",
        target.singular_api_name,
        uppercase_first_letter(&source.plural_api_name)
    );

    Ok(LinkDefinition {
        const_name: api_name.clone(),
        api_name,
        one: LinkEndpointDefinition {
            object_const_name: target.object_api_name.clone(),
            api_name: source.plural_api_name.clone(),
            display_name: source.display_name.clone(),
            plural_display_name: source.plural_display_name.clone(),
        },
        to_many: LinkEndpointDefinition {
            object_const_name: source.object_api_name.clone(),
            api_name: target.singular_api_name.clone(),
            display_name: target.display_name.clone(),
            plural_display_name: target.plural_display_name.clone(),
        },
        many_foreign_key_property: to_property_api_name(source_column),
    })
}

fn infer_primary_key_column<'a>(
    table: &'a Table,
    incoming_foreign_key_targets: &[&str],
    outgoing_foreign_key_sources: &[&str],
) -> Result<PrimaryKeySelection<'a>> {
    match table.primary_key.as_slice() {
        [name] => return Ok(PrimaryKeySelection::declared(name)),
        [] => {}
        _ => {
            return Ok(PrimaryKeySelection::synthetic_composite(
                table.primary_key.as_slice(),
            ));
        }
    }

    let referenced_targets = unique_non_nullable_columns(table, incoming_foreign_key_targets);
    if referenced_targets.len() == 1 {
        return Ok(PrimaryKeySelection::inferred(referenced_targets[0]));
    }

    let singular_table_name = singularize_identifier(&table.name);
    let table_suffix = singular_table_name
        .rsplit('_')
        .next()
        .unwrap_or(singular_table_name.as_str());
    let exact_candidates = [
        "id".to_owned(),
        format!("{}_id", table.name),
        format!("{}_id", singular_table_name),
        format!("{}_id", table_suffix),
        "num".to_owned(),
        format!("{}_num", table.name),
        format!("{}_num", singular_table_name),
        format!("{}_num", table_suffix),
        "code".to_owned(),
        format!("{}_code", table.name),
        format!("{}_code", singular_table_name),
        format!("{}_code", table_suffix),
    ];

    for candidate in exact_candidates {
        if let Some(column) = table.columns.iter().find(|column| {
            !column.nullable
                && column.name.eq_ignore_ascii_case(&candidate)
                && !contains_identifier(outgoing_foreign_key_sources, &column.name)
        }) {
            return Ok(PrimaryKeySelection::inferred(column.name.as_str()));
        }
    }

    if let Some(column) = table.columns.iter().find(|column| {
        !column.nullable
            && (column.name.eq_ignore_ascii_case("id")
                || column.name.to_ascii_lowercase().ends_with("_id"))
            && !contains_identifier(outgoing_foreign_key_sources, &column.name)
    }) {
        return Ok(PrimaryKeySelection::inferred(column.name.as_str()));
    }

    if let Some(column) = table.columns.iter().find(|column| {
        !column.nullable
            && (column.name.eq_ignore_ascii_case("num")
                || column.name.eq_ignore_ascii_case("code")
                || column.name.to_ascii_lowercase().ends_with("_num")
                || column.name.to_ascii_lowercase().ends_with("_code")
                || column.name.to_ascii_lowercase().ends_with("_key"))
            && !contains_identifier(outgoing_foreign_key_sources, &column.name)
    }) {
        return Ok(PrimaryKeySelection::inferred(column.name.as_str()));
    }

    if let Some(column) = table.columns.iter().find(|column| {
        !column.nullable
            && ["id", "num", "code", "key"]
                .iter()
                .any(|needle| column.name.to_ascii_lowercase().contains(needle))
            && !contains_identifier(outgoing_foreign_key_sources, &column.name)
    }) {
        return Ok(PrimaryKeySelection::inferred(column.name.as_str()));
    }

    if let Some(column) = table.columns.iter().find(|column| !column.nullable) {
        return Ok(PrimaryKeySelection::inferred(column.name.as_str()));
    }

    bail!("table '{}' does not define a primary key", table.name)
}

fn unique_non_nullable_columns<'a>(table: &'a Table, candidates: &[&str]) -> Vec<&'a str> {
    let mut unique = Vec::new();

    for candidate in candidates {
        if unique
            .iter()
            .any(|existing: &&str| existing.eq_ignore_ascii_case(candidate))
        {
            continue;
        }

        if let Some(column) = table
            .columns
            .iter()
            .find(|column| !column.nullable && column.name.eq_ignore_ascii_case(candidate))
        {
            unique.push(column.name.as_str());
        }
    }

    unique
}

fn contains_identifier(candidates: &[&str], name: &str) -> bool {
    candidates
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(name))
}

fn choose_title_property(
    properties: &[PropertyDefinition],
    primary_key_property: &str,
) -> Option<String> {
    let preferred_names = ["name", "title", "label", "code", "number"];

    for preferred_name in preferred_names {
        if let Some(property) = properties.iter().find(|property| {
            property.osdk_type == "string"
                && property.api_name != primary_key_property
                && property
                    .api_name
                    .to_ascii_lowercase()
                    .contains(preferred_name)
        }) {
            return Some(property.api_name.clone());
        }
    }

    properties
        .iter()
        .find(|property| {
            property.osdk_type == "string" && property.api_name != primary_key_property
        })
        .or_else(|| {
            properties
                .iter()
                .find(|property| property.api_name != primary_key_property)
        })
        .or_else(|| properties.first())
        .map(|property| property.api_name.clone())
}

fn to_property_api_name(name: &str) -> String {
    sanitize_identifier(&name.to_case(Case::Camel))
}

fn sanitize_identifier(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return "_".to_owned();
    };

    let mut identifier = String::with_capacity(value.len());
    if first.is_ascii_alphabetic() || first == '_' {
        identifier.push(first);
    } else {
        identifier.push('_');
        if first.is_ascii_alphanumeric() {
            identifier.push(first);
        }
    }

    for ch in chars {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            identifier.push(ch);
        }
    }

    if identifier.is_empty() {
        "_".to_owned()
    } else {
        identifier
    }
}

fn uppercase_first_letter(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };

    let mut output = String::new();
    output.extend(first.to_uppercase());
    output.extend(chars);
    output
}

fn entity_names(raw_table_name: &str) -> EntityNames {
    let singular_raw_name = singularize_identifier(raw_table_name);
    let plural_raw_name = if singular_raw_name == raw_table_name {
        pluralize_identifier(raw_table_name)
    } else {
        raw_table_name.to_owned()
    };

    EntityNames {
        object_api_name: sanitize_identifier(&raw_table_name.to_case(Case::Camel)),
        singular_api_name: sanitize_identifier(&singular_raw_name.to_case(Case::Camel)),
        plural_api_name: sanitize_identifier(&plural_raw_name.to_case(Case::Camel)),
        display_name: singular_raw_name.to_case(Case::Title),
        plural_display_name: plural_raw_name.to_case(Case::Title),
    }
}

fn singularize_identifier(identifier: &str) -> String {
    let mut parts = identifier
        .split('_')
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    if let Some(last) = parts.last_mut() {
        *last = singularize_word(last);
    }

    parts.join("_")
}

fn pluralize_identifier(identifier: &str) -> String {
    let mut parts = identifier
        .split('_')
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    if let Some(last) = parts.last_mut() {
        *last = pluralize_word(last);
    }

    parts.join("_")
}

fn singularize_word(word: &str) -> String {
    let lower = word.to_ascii_lowercase();

    if is_uncountable(&lower) {
        word.to_owned()
    } else if lower.ends_with("ies") && word.len() > 3 {
        format!("{}y", &word[..word.len() - 3])
    } else if lower.ends_with("ches")
        || lower.ends_with("shes")
        || lower.ends_with("sses")
        || lower.ends_with("xes")
        || lower.ends_with("zes")
    {
        word[..word.len() - 2].to_owned()
    } else if lower.ends_with('s') && !lower.ends_with("ss") && word.len() > 1 {
        word[..word.len() - 1].to_owned()
    } else {
        word.to_owned()
    }
}

fn pluralize_word(word: &str) -> String {
    let lower = word.to_ascii_lowercase();

    if is_uncountable(&lower) {
        word.to_owned()
    } else if lower.ends_with('y')
        && !lower.ends_with("ay")
        && !lower.ends_with("ey")
        && !lower.ends_with("iy")
        && !lower.ends_with("oy")
        && !lower.ends_with("uy")
    {
        format!("{}ies", &word[..word.len() - 1])
    } else if lower.ends_with('s')
        || lower.ends_with('x')
        || lower.ends_with('z')
        || lower.ends_with("ch")
        || lower.ends_with("sh")
    {
        format!("{word}es")
    } else {
        format!("{word}s")
    }
}

fn is_uncountable(word: &str) -> bool {
    matches!(word, "data" | "metadata")
}

fn map_sql_type(sql_type: &str) -> &'static str {
    let normalized = sql_type.trim().to_ascii_lowercase();

    if normalized == "uuid"
        || normalized == "text"
        || normalized == "citext"
        || normalized.starts_with("varchar")
        || normalized.starts_with("character varying")
        || normalized.starts_with("char(")
        || normalized.starts_with("character(")
        || normalized == "character"
    {
        "string"
    } else if normalized == "smallint"
        || normalized == "int2"
        || normalized == "integer"
        || normalized == "int"
        || normalized == "int4"
        || normalized == "serial"
        || normalized == "smallserial"
    {
        "integer"
    } else if normalized == "bigint" || normalized == "int8" || normalized == "bigserial" {
        "long"
    } else if normalized == "numeric"
        || normalized.starts_with("numeric(")
        || normalized == "decimal"
        || normalized.starts_with("decimal(")
        || normalized == "real"
        || normalized == "float4"
        || normalized == "float8"
        || normalized == "double precision"
        || normalized.starts_with("double precision")
    {
        "double"
    } else if normalized == "boolean" || normalized == "bool" {
        "boolean"
    } else if normalized == "date" {
        "date"
    } else if normalized.starts_with("timestamp")
        || normalized == "timestamptz"
        || normalized.starts_with("timestamp with time zone")
        || normalized.starts_with("timestamp without time zone")
    {
        "timestamp"
    } else {
        "string"
    }
}

#[derive(Debug, Clone)]
struct EntityNames {
    object_api_name: String,
    singular_api_name: String,
    plural_api_name: String,
    display_name: String,
    plural_display_name: String,
}

#[derive(Debug, Clone)]
struct PrimaryKeySelection<'a> {
    #[allow(dead_code)]
    column_name: &'a str,
    property_api_name: String,
    synthetic_property: Option<PropertyDefinition>,
    warning: Option<String>,
}

impl<'a> PrimaryKeySelection<'a> {
    fn declared(column_name: &'a str) -> Self {
        Self {
            column_name,
            property_api_name: to_property_api_name(column_name),
            synthetic_property: None,
            warning: None,
        }
    }

    fn inferred(column_name: &'a str) -> Self {
        Self {
            column_name,
            property_api_name: to_property_api_name(column_name),
            synthetic_property: None,
            warning: Some(format!(
                "has no declared primary key; using '{}' as an inferred primary key",
                column_name
            )),
        }
    }

    fn synthetic_composite(columns: &'a [String]) -> Self {
        let mut api_name = String::new();
        for (index, column) in columns.iter().enumerate() {
            let component = to_property_api_name(column);
            if index == 0 {
                api_name.push_str(&component);
            } else {
                api_name.push_str(&uppercase_first_letter(&component));
            }
        }
        api_name.push_str("Key");

        let display_name = format!(
            "{} Key",
            columns
                .iter()
                .map(|column| column.to_case(Case::Title))
                .collect::<Vec<_>>()
                .join(" / ")
        );

        Self {
            column_name: columns
                .first()
                .map(String::as_str)
                .unwrap_or("composite_key"),
            property_api_name: api_name.clone(),
            synthetic_property: Some(PropertyDefinition {
                api_name,
                display_name,
                osdk_type: "string".to_owned(),
            }),
            warning: Some(format!(
                "uses composite primary key ({}); generated synthetic string primary key",
                columns.join(", ")
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::schema::{Column, Table};

    use super::{entity_names, infer_primary_key_column, map_table, singularize_identifier};

    #[test]
    fn chooses_string_title_property_before_primary_key() {
        let table = Table {
            name: "employee".to_owned(),
            columns: vec![
                Column {
                    name: "id".to_owned(),
                    sql_type: "uuid".to_owned(),
                    nullable: false,
                    default: None,
                },
                Column {
                    name: "full_name".to_owned(),
                    sql_type: "text".to_owned(),
                    nullable: false,
                    default: None,
                },
            ],
            primary_key: vec!["id".to_owned()],
        };

        let primary_key = infer_primary_key_column(&table, &[], &[]).expect("pk should resolve");
        let object =
            map_table(&table, &entity_names(&table.name), &primary_key).expect("table should map");
        assert_eq!(object.title_property_api_name, "fullName");
    }

    #[test]
    fn singularizes_plural_table_names_for_display() {
        let names = entity_names("software_part_numbers");
        assert_eq!(names.object_api_name, "softwarePartNumbers");
        assert_eq!(names.singular_api_name, "softwarePartNumber");
        assert_eq!(names.display_name, "Software Part Number");
        assert_eq!(names.plural_display_name, "Software Part Numbers");
        assert_eq!(singularize_identifier("map_data"), "map_data");
    }

    #[test]
    fn infers_primary_key_from_not_null_identifier_column() {
        let table = Table {
            name: "netflix_shows".to_owned(),
            columns: vec![
                Column {
                    name: "show_id".to_owned(),
                    sql_type: "text".to_owned(),
                    nullable: false,
                    default: None,
                },
                Column {
                    name: "title".to_owned(),
                    sql_type: "text".to_owned(),
                    nullable: true,
                    default: None,
                },
            ],
            primary_key: vec![],
        };

        let primary_key =
            infer_primary_key_column(&table, &[], &[]).expect("pk should be inferred");
        assert_eq!(primary_key.column_name, "show_id");
        assert!(primary_key.warning.is_some());

        let object =
            map_table(&table, &entity_names(&table.name), &primary_key).expect("table should map");
        assert_eq!(object.primary_key_property_api_name, "showId");
    }

    #[test]
    fn infers_primary_key_from_not_null_num_column() {
        let table = Table {
            name: "lego_sets".to_owned(),
            columns: vec![
                Column {
                    name: "set_num".to_owned(),
                    sql_type: "character varying(255)".to_owned(),
                    nullable: false,
                    default: None,
                },
                Column {
                    name: "name".to_owned(),
                    sql_type: "character varying(255)".to_owned(),
                    nullable: false,
                    default: None,
                },
            ],
            primary_key: vec![],
        };

        let primary_key =
            infer_primary_key_column(&table, &[], &[]).expect("pk should be inferred");
        assert_eq!(primary_key.column_name, "set_num");
        assert!(primary_key.warning.is_some());

        let object =
            map_table(&table, &entity_names(&table.name), &primary_key).expect("table should map");
        assert_eq!(object.primary_key_property_api_name, "setNum");
    }

    #[test]
    fn prefers_table_suffix_num_over_foreign_key_id() {
        let table = Table {
            name: "lego_parts".to_owned(),
            columns: vec![
                Column {
                    name: "part_num".to_owned(),
                    sql_type: "character varying(255)".to_owned(),
                    nullable: false,
                    default: None,
                },
                Column {
                    name: "name".to_owned(),
                    sql_type: "text".to_owned(),
                    nullable: false,
                    default: None,
                },
                Column {
                    name: "part_cat_id".to_owned(),
                    sql_type: "integer".to_owned(),
                    nullable: false,
                    default: None,
                },
            ],
            primary_key: vec![],
        };

        let primary_key =
            infer_primary_key_column(&table, &[], &["part_cat_id"]).expect("pk should be inferred");
        assert_eq!(primary_key.column_name, "part_num");
        assert!(primary_key.warning.is_some());
    }

    #[test]
    fn prefers_incoming_foreign_key_target_before_name_heuristics() {
        let table = Table {
            name: "lego_sets".to_owned(),
            columns: vec![
                Column {
                    name: "set_num".to_owned(),
                    sql_type: "character varying(255)".to_owned(),
                    nullable: false,
                    default: None,
                },
                Column {
                    name: "theme_id".to_owned(),
                    sql_type: "integer".to_owned(),
                    nullable: false,
                    default: None,
                },
            ],
            primary_key: vec![],
        };

        let primary_key = infer_primary_key_column(&table, &["set_num"], &["theme_id"])
            .expect("pk should be inferred from incoming fk");
        assert_eq!(primary_key.column_name, "set_num");
        assert!(primary_key.warning.is_some());
    }

    #[test]
    fn synthesizes_property_for_composite_primary_key() {
        let table = Table {
            name: "playlist_track".to_owned(),
            columns: vec![
                Column {
                    name: "PlaylistId".to_owned(),
                    sql_type: "integer".to_owned(),
                    nullable: false,
                    default: None,
                },
                Column {
                    name: "TrackId".to_owned(),
                    sql_type: "integer".to_owned(),
                    nullable: false,
                    default: None,
                },
            ],
            primary_key: vec!["PlaylistId".to_owned(), "TrackId".to_owned()],
        };

        let primary_key = infer_primary_key_column(&table, &[], &[]).expect("pk should resolve");
        assert_eq!(primary_key.property_api_name, "playlistIdTrackIdKey");
        assert!(primary_key.synthetic_property.is_some());
        assert!(primary_key.warning.is_some());

        let object =
            map_table(&table, &entity_names(&table.name), &primary_key).expect("table should map");
        assert_eq!(object.primary_key_property_api_name, "playlistIdTrackIdKey");
        assert!(
            object
                .properties
                .iter()
                .any(|property| property.api_name == "playlistIdTrackIdKey")
        );
    }
}
