use std::collections::HashMap;

use anyhow::{Result, anyhow, bail};
use convert_case::{Case, Casing};

use crate::ontology::{
    ActionDefinition, ActionKind, ActionParameterDefinition, ActionParameterTypeDefinition,
    ActionPropertyMapping, LinkDefinition, LinkEndpointDefinition, ModuleDefinition,
    ObjectDefinition, PropertyDefinition, ValueTypeDefinition,
};
use crate::schema::{
    Column, DatabaseSchema, ForeignKey, SqlEnumType, SqlRoutine, SqlRoutineArg, Table,
};

pub fn map_schema_to_ontology(schema: &DatabaseSchema) -> Result<ModuleDefinition> {
    let mut objects = Vec::with_capacity(schema.tables.len());
    let value_types = schema
        .enum_types
        .iter()
        .map(map_enum_type_to_value_type)
        .collect::<Vec<_>>();
    let enum_value_types_by_sql_type = schema
        .enum_types
        .iter()
        .zip(value_types.iter())
        .map(|(enum_type, value_type)| {
            (
                normalize_sql_type_name(&enum_type.name),
                value_type.const_name.clone(),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut entity_names_by_table = HashMap::with_capacity(schema.tables.len());
    let mut object_definitions_by_table = HashMap::with_capacity(schema.tables.len());
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

        let object = map_table(
            table,
            &entity_names,
            &primary_key,
            &enum_value_types_by_sql_type,
        )?;
        object_definitions_by_table.insert(table.name.clone(), object.clone());
        objects.push(object);
    }

    let links = schema
        .foreign_keys
        .iter()
        .map(|foreign_key| map_foreign_key(foreign_key, &entity_names_by_table))
        .collect::<Result<Vec<_>>>()?;

    let mut actions = Vec::new();
    for routine in &schema.routines {
        match map_routine_to_action(routine, &object_definitions_by_table)? {
            Some(action) => actions.push(action),
            None => warnings.push(format!(
                "routine '{}': read-only routines are not emitted as ontology actions",
                routine.name
            )),
        }
    }

    Ok(ModuleDefinition {
        value_types,
        objects,
        links,
        actions,
        warnings,
    })
}

fn map_table(
    table: &Table,
    entity_names: &EntityNames,
    primary_key: &PrimaryKeySelection<'_>,
    enum_value_types_by_sql_type: &HashMap<String, String>,
) -> Result<ObjectDefinition> {
    let mut properties: Vec<PropertyDefinition> = table
        .columns
        .iter()
        .map(|column| map_column(column, enum_value_types_by_sql_type))
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

fn map_column(
    column: &Column,
    enum_value_types_by_sql_type: &HashMap<String, String>,
) -> Result<PropertyDefinition> {
    let api_name = to_property_api_name(&column.name);
    let display_name = column.name.to_case(Case::Title);
    let value_type_const_name =
        enum_value_types_by_sql_type.get(&normalize_sql_type_name(&column.sql_type));
    let osdk_type = if value_type_const_name.is_some() {
        "string".to_owned()
    } else {
        map_sql_type(&column.sql_type).to_owned()
    };

    Ok(PropertyDefinition {
        api_name,
        display_name,
        osdk_type,
        value_type_const_name: value_type_const_name.cloned(),
    })
}

fn map_enum_type_to_value_type(enum_type: &SqlEnumType) -> ValueTypeDefinition {
    let api_name = sanitize_identifier(&enum_type.name.to_case(Case::Camel));
    ValueTypeDefinition {
        const_name: format!("{}ValueType", api_name),
        api_name,
        display_name: enum_type.name.to_case(Case::Title),
        values: enum_type.values.clone(),
    }
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

fn map_routine_to_action(
    routine: &SqlRoutine,
    objects_by_table: &HashMap<String, ObjectDefinition>,
) -> Result<Option<ActionDefinition>> {
    if let Some(table_name) = routine.name.strip_prefix("create_") {
        let object = objects_by_table.get(table_name).ok_or_else(|| {
            anyhow!(
                "routine '{}' references unknown table '{}'",
                routine.name,
                table_name
            )
        })?;

        let property_mappings = routine
            .args
            .iter()
            .map(|arg| {
                Ok(ActionPropertyMapping {
                    property_api_name: resolve_property_api_name(object, &arg.name, None)?,
                    parameter_id: routine_parameter_id(&arg.name),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let parameters = routine
            .args
            .iter()
            .map(|arg| map_routine_arg_to_parameter(arg, object))
            .collect::<Vec<_>>();

        return Ok(Some(ActionDefinition {
            const_name: sanitize_identifier(&routine.name.to_case(Case::Camel)),
            api_name: to_action_api_name(&routine.name),
            display_name: routine_display_name(&routine.name),
            object_const_name: object.const_name.clone(),
            object_api_name: object.api_name.clone(),
            kind: ActionKind::Create {
                parameters,
                property_mappings,
            },
        }));
    }

    if let Some(table_name) = routine.name.strip_prefix("update_") {
        let object = objects_by_table.get(table_name).ok_or_else(|| {
            anyhow!(
                "routine '{}' references unknown table '{}'",
                routine.name,
                table_name
            )
        })?;
        let primary_key_arg = routine.args.first().ok_or_else(|| {
            anyhow!(
                "routine '{}' does not define a target key argument",
                routine.name
            )
        })?;

        let property_mappings = routine
            .args
            .iter()
            .skip(1)
            .map(|arg| {
                Ok(ActionPropertyMapping {
                    property_api_name: resolve_property_api_name(
                        object,
                        &arg.name,
                        arg.name.strip_prefix("p_new_"),
                    )?,
                    parameter_id: routine_parameter_id(&arg.name),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let mut parameters = Vec::with_capacity(routine.args.len());
        parameters.push(target_object_parameter(object));
        parameters.extend(
            routine
                .args
                .iter()
                .skip(1)
                .map(|arg| map_routine_arg_to_parameter(arg, object)),
        );

        if !matches_target_primary_key_arg(primary_key_arg, object) {
            bail!(
                "routine '{}' does not use its first argument as the primary key for table '{}'",
                routine.name,
                table_name
            );
        }

        return Ok(Some(ActionDefinition {
            const_name: sanitize_identifier(&routine.name.to_case(Case::Camel)),
            api_name: to_action_api_name(&routine.name),
            display_name: routine_display_name(&routine.name),
            object_const_name: object.const_name.clone(),
            object_api_name: object.api_name.clone(),
            kind: ActionKind::Modify {
                parameters,
                property_mappings,
            },
        }));
    }

    if let Some(table_name) = routine.name.strip_prefix("delete_") {
        let object = objects_by_table.get(table_name).ok_or_else(|| {
            anyhow!(
                "routine '{}' references unknown table '{}'",
                routine.name,
                table_name
            )
        })?;

        if let Some(primary_key_arg) = routine.args.first()
            && !matches_target_primary_key_arg(primary_key_arg, object)
        {
            bail!(
                "routine '{}' does not use its first argument as the primary key for table '{}'",
                routine.name,
                table_name
            );
        }

        return Ok(Some(ActionDefinition {
            const_name: sanitize_identifier(&routine.name.to_case(Case::Camel)),
            api_name: to_action_api_name(&routine.name),
            display_name: routine_display_name(&routine.name),
            object_const_name: object.const_name.clone(),
            object_api_name: object.api_name.clone(),
            kind: ActionKind::Delete,
        }));
    }

    if let Some(rest) = routine.name.strip_prefix("set_") {
        let (table_name, property_name) = split_set_routine_name(rest, objects_by_table)
            .ok_or_else(|| {
                anyhow!(
                    "routine '{}' does not match set_<table>_<property> naming",
                    routine.name
                )
            })?;
        let object = objects_by_table.get(table_name).ok_or_else(|| {
            anyhow!(
                "routine '{}' references unknown table '{}'",
                routine.name,
                table_name
            )
        })?;
        let primary_key_arg = routine.args.first().ok_or_else(|| {
            anyhow!(
                "routine '{}' does not define a target key argument",
                routine.name
            )
        })?;

        if !matches_target_primary_key_arg(primary_key_arg, object) {
            bail!(
                "routine '{}' does not use its first argument as the primary key for table '{}'",
                routine.name,
                table_name
            );
        }

        let value_arg = routine.args.get(1).ok_or_else(|| {
            anyhow!(
                "routine '{}' does not define a value argument",
                routine.name
            )
        })?;

        let parameters = vec![
            target_object_parameter(object),
            map_routine_arg_to_parameter(value_arg, object),
        ];
        let property_mappings = vec![ActionPropertyMapping {
            property_api_name: resolve_property_api_name(
                object,
                &value_arg.name,
                Some(property_name),
            )?,
            parameter_id: routine_parameter_id(&value_arg.name),
        }];

        return Ok(Some(ActionDefinition {
            const_name: sanitize_identifier(&routine.name.to_case(Case::Camel)),
            api_name: to_action_api_name(&routine.name),
            display_name: routine_display_name(&routine.name),
            object_const_name: object.const_name.clone(),
            object_api_name: object.api_name.clone(),
            kind: ActionKind::Modify {
                parameters,
                property_mappings,
            },
        }));
    }

    if routine.name.starts_with("get_") || routine.name.starts_with("list_") {
        return Ok(None);
    }

    Ok(None)
}

fn split_set_routine_name<'a>(
    rest: &'a str,
    objects_by_table: &'a HashMap<String, ObjectDefinition>,
) -> Option<(&'a str, &'a str)> {
    let mut candidates = objects_by_table
        .keys()
        .filter_map(|table_name| {
            let prefix = format!("{table_name}_");
            rest.strip_prefix(&prefix)
                .map(|property_name| (table_name.as_str(), property_name))
        })
        .collect::<Vec<_>>();

    candidates.sort_by_key(|(table_name, _)| std::cmp::Reverse(table_name.len()));
    candidates.into_iter().next()
}

fn resolve_property_api_name(
    object: &ObjectDefinition,
    routine_arg_name: &str,
    override_column_name: Option<&str>,
) -> Result<String> {
    let candidate_names = [
        override_column_name.map(ToOwned::to_owned),
        Some(strip_routine_arg_prefix(routine_arg_name).to_owned()),
        strip_routine_arg_prefix(routine_arg_name)
            .strip_prefix("new_")
            .map(ToOwned::to_owned),
    ];

    for candidate in candidate_names.into_iter().flatten() {
        let property_api_name = to_property_api_name(&candidate);
        if object
            .properties
            .iter()
            .any(|property| property.api_name == property_api_name)
        {
            return Ok(property_api_name);
        }
    }

    bail!(
        "routine argument '{}' does not match any property on object '{}'",
        routine_arg_name,
        object.api_name
    )
}

fn strip_routine_arg_prefix(name: &str) -> &str {
    name.strip_prefix("p_").unwrap_or(name)
}

fn routine_parameter_id(name: &str) -> String {
    sanitize_identifier(&strip_routine_arg_prefix(name).to_case(Case::Camel))
}

fn target_object_parameter(object: &ObjectDefinition) -> ActionParameterDefinition {
    ActionParameterDefinition {
        id: "objectToModifyParameter".to_owned(),
        display_name: "Modify Object".to_owned(),
        parameter_type: ActionParameterTypeDefinition::ObjectReference {
            object_api_name: object.api_name.clone(),
        },
        required: true,
    }
}

fn map_routine_arg_to_parameter(
    arg: &SqlRoutineArg,
    _object: &ObjectDefinition,
) -> ActionParameterDefinition {
    ActionParameterDefinition {
        id: routine_parameter_id(&arg.name),
        display_name: strip_routine_arg_prefix(&arg.name).to_case(Case::Title),
        parameter_type: ActionParameterTypeDefinition::Primitive(
            map_sql_type(&arg.sql_type).to_owned(),
        ),
        required: !arg.has_default,
    }
}

fn matches_target_primary_key_arg(arg: &SqlRoutineArg, object: &ObjectDefinition) -> bool {
    routine_parameter_id(&arg.name) == object.primary_key_property_api_name
}

fn routine_display_name(name: &str) -> String {
    name.replace('_', " ").to_case(Case::Title)
}

fn to_action_api_name(name: &str) -> String {
    sanitize_action_api_name(&name.to_case(Case::Kebab))
}

fn sanitize_action_api_name(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut last_was_dash = false;

    for ch in value.chars() {
        let normalized = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else {
            '-'
        };

        if normalized == '-' {
            if !output.is_empty() && !last_was_dash {
                output.push('-');
            }
            last_was_dash = true;
        } else {
            output.push(normalized);
            last_was_dash = false;
        }
    }

    while output.ends_with('-') {
        output.pop();
    }

    if output.is_empty() {
        "action".to_owned()
    } else {
        output
    }
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

fn normalize_sql_type_name(name: &str) -> String {
    name.trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_ascii_lowercase()
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
                value_type_const_name: None,
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
    use std::collections::HashMap;

    use crate::schema::{Column, DatabaseSchema, SqlRoutine, SqlRoutineArg, Table};

    use super::{
        ActionKind, entity_names, infer_primary_key_column, map_schema_to_ontology, map_table,
        singularize_identifier,
    };

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
        let object = map_table(
            &table,
            &entity_names(&table.name),
            &primary_key,
            &HashMap::new(),
        )
        .expect("table should map");
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

        let object = map_table(
            &table,
            &entity_names(&table.name),
            &primary_key,
            &HashMap::new(),
        )
        .expect("table should map");
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

        let object = map_table(
            &table,
            &entity_names(&table.name),
            &primary_key,
            &HashMap::new(),
        )
        .expect("table should map");
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

        let object = map_table(
            &table,
            &entity_names(&table.name),
            &primary_key,
            &HashMap::new(),
        )
        .expect("table should map");
        assert_eq!(object.primary_key_property_api_name, "playlistIdTrackIdKey");
        assert!(
            object
                .properties
                .iter()
                .any(|property| property.api_name == "playlistIdTrackIdKey")
        );
    }

    #[test]
    fn maps_update_routine_to_modify_action_with_primary_key_reassignment() {
        let schema = DatabaseSchema {
            tables: vec![Table {
                name: "asset".to_owned(),
                columns: vec![
                    Column {
                        name: "asset_id".to_owned(),
                        sql_type: "varchar(20)".to_owned(),
                        nullable: false,
                        default: None,
                    },
                    Column {
                        name: "vin".to_owned(),
                        sql_type: "varchar(17)".to_owned(),
                        nullable: false,
                        default: None,
                    },
                ],
                primary_key: vec!["asset_id".to_owned()],
            }],
            foreign_keys: vec![],
            routines: vec![SqlRoutine {
                name: "update_asset".to_owned(),
                args: vec![
                    SqlRoutineArg {
                        name: "p_asset_id".to_owned(),
                        sql_type: "varchar(20)".to_owned(),
                        has_default: false,
                    },
                    SqlRoutineArg {
                        name: "p_new_asset_id".to_owned(),
                        sql_type: "varchar(20)".to_owned(),
                        has_default: false,
                    },
                    SqlRoutineArg {
                        name: "p_vin".to_owned(),
                        sql_type: "varchar(17)".to_owned(),
                        has_default: false,
                    },
                ],
                return_type: Some("asset".to_owned()),
            }],
            enum_types: vec![],
        };

        let module = map_schema_to_ontology(&schema).expect("schema should map");
        assert_eq!(module.actions.len(), 1);
        let rendered_target = &module.actions[0];
        assert_eq!(rendered_target.api_name, "update-asset");

        match &rendered_target.kind {
            ActionKind::Modify {
                parameters,
                property_mappings,
            } => {
                assert_eq!(parameters[0].id, "objectToModifyParameter");
                assert!(
                    property_mappings
                        .iter()
                        .any(|mapping| mapping.property_api_name == "assetId"
                            && mapping.parameter_id == "newAssetId")
                );
                assert!(
                    property_mappings
                        .iter()
                        .any(|mapping| mapping.property_api_name == "vin"
                            && mapping.parameter_id == "vin")
                );
            }
            _ => panic!("expected modify action"),
        }
    }
}
