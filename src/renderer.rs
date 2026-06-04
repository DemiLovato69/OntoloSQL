use crate::ontology::{
    ActionDefinition, ActionKind, ActionParameterDefinition, ActionParameterTypeDefinition,
    LinkDefinition, LinkEndpointDefinition, ModuleDefinition, ObjectDefinition, PropertyDefinition,
    ValueTypeDefinition,
};

pub fn render_module(module: &ModuleDefinition) -> String {
    let import = render_import(module);

    let mut output = import;

    for (index, value_type) in module.value_types.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        output.push_str(&render_value_type(value_type));
    }

    if !module.value_types.is_empty() && !module.objects.is_empty() {
        output.push('\n');
    }

    for (index, object) in module.objects.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        output.push_str(&render_object(object));
    }

    if !module.links.is_empty() {
        output.push('\n');
    }

    for (index, link) in module.links.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        output.push_str(&render_link(link));
    }

    if !module.actions.is_empty() {
        output.push('\n');
    }

    for (index, action) in module.actions.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        output.push_str(&render_action(action));
    }

    output
}

fn render_import(module: &ModuleDefinition) -> String {
    let mut imports = vec!["defineObject"];

    if !module.value_types.is_empty() {
        imports.push("defineValueType");
    }

    if !module.links.is_empty() {
        imports.push("defineLink");
    }

    if !module.actions.is_empty() {
        imports.push("defineAction");
        if module
            .actions
            .iter()
            .any(|action| matches!(action.kind, ActionKind::Modify { .. }))
        {
            imports.push("MODIFY_OBJECT_PARAMETER");
        }
        if module
            .actions
            .iter()
            .any(|action| matches!(action.kind, ActionKind::Delete))
        {
            imports.push("DELETE_OBJECT_PARAMETER");
        }
    }

    imports.sort_unstable();
    imports.dedup();

    format!(
        "import {{ {} }} from \"@osdk/maker\";\n\n",
        imports.join(", ")
    )
}

fn render_value_type(value_type: &ValueTypeDefinition) -> String {
    let allowed_values = value_type
        .values
        .iter()
        .map(|value| format!("\"{}\"", escape_ts_string(value)))
        .collect::<Vec<_>>()
        .join(", ");

    let mut output = String::new();
    output.push_str(&format!(
        "export const {} = defineValueType({{\n",
        value_type.const_name
    ));
    output.push_str(&format!("  apiName: \"{}\",\n", value_type.api_name));
    output.push_str(&format!(
        "  displayName: \"{}\",\n",
        escape_ts_string(&value_type.display_name)
    ));
    output.push_str("  type: {\n");
    output.push_str("    type: \"string\",\n");
    output.push_str("    constraints: [{\n");
    output.push_str("      constraint: {\n");
    output.push_str(&format!("        allowedValues: [{}],\n", allowed_values));
    output.push_str("      },\n");
    output.push_str("    }],\n");
    output.push_str("  },\n");
    output.push_str("  version: \"0.1.0\",\n");
    output.push_str("});\n");
    output
}

fn render_object(object: &ObjectDefinition) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "export const {} = defineObject({{\n",
        object.const_name
    ));
    output.push_str(&format!("  apiName: \"{}\",\n", object.api_name));
    output.push_str(&format!("  displayName: \"{}\",\n", object.display_name));
    output.push_str(&format!(
        "  pluralDisplayName: \"{}\",\n",
        object.plural_display_name
    ));
    output.push_str(&format!(
        "  titlePropertyApiName: \"{}\",\n",
        object.title_property_api_name
    ));
    output.push_str(&format!(
        "  primaryKeyPropertyApiName: \"{}\",\n",
        object.primary_key_property_api_name
    ));
    output.push_str("  properties: {\n");

    for property in &object.properties {
        output.push_str(&render_property(property));
    }

    output.push_str("  },\n");
    output.push_str("});\n");
    output
}

fn render_property(property: &PropertyDefinition) -> String {
    let value_type = property
        .value_type_const_name
        .as_ref()
        .map(|const_name| format!(", valueType: {}", const_name))
        .unwrap_or_default();
    format!(
        "    \"{}\": {{ type: \"{}\", displayName: \"{}\"{} }},\n",
        property.api_name, property.osdk_type, property.display_name, value_type
    )
}

fn escape_ts_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn render_link(link: &LinkDefinition) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "export const {} = defineLink({{\n",
        link.const_name
    ));
    output.push_str(&format!("  apiName: \"{}\",\n", link.api_name));
    output.push_str("  one: {\n");
    output.push_str(&render_link_endpoint(&link.one));
    output.push_str("  },\n");
    output.push_str("  toMany: {\n");
    output.push_str(&render_link_endpoint(&link.to_many));
    output.push_str("  },\n");
    output.push_str(&format!(
        "  manyForeignKeyProperty: \"{}\",\n",
        link.many_foreign_key_property
    ));
    output.push_str("});\n");
    output
}

fn render_link_endpoint(endpoint: &LinkEndpointDefinition) -> String {
    let mut output = String::new();
    output.push_str(&format!("    object: {},\n", endpoint.object_const_name));
    output.push_str("    metadata: {\n");
    output.push_str(&format!("      apiName: \"{}\",\n", endpoint.api_name));
    output.push_str(&format!(
        "      displayName: \"{}\",\n",
        endpoint.display_name
    ));
    output.push_str(&format!(
        "      pluralDisplayName: \"{}\",\n",
        endpoint.plural_display_name
    ));
    output.push_str("    },\n");
    output
}

fn render_action(action: &ActionDefinition) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "export const {} = defineAction({{\n",
        action.const_name
    ));
    output.push_str(&format!("  apiName: \"{}\",\n", action.api_name));
    output.push_str(&format!("  displayName: \"{}\",\n", action.display_name));
    output.push_str(&format!("  status: \"active\",\n"));
    output.push_str("  parameters: [\n");

    match &action.kind {
        ActionKind::Create { parameters, .. } | ActionKind::Modify { parameters, .. } => {
            for parameter in parameters {
                output.push_str(&render_action_parameter(parameter));
            }
        }
        ActionKind::Delete => {
            output.push_str(&render_delete_target_parameter(&action.object_api_name));
        }
    }

    output.push_str("  ],\n");
    output.push_str("  rules: [\n");
    output.push_str(&render_action_rule(action));
    output.push_str("  ],\n");
    output.push_str("  entities: {\n");
    output.push_str("    affectedInterfaceTypes: [],\n");
    output.push_str(&format!(
        "    affectedObjectTypes: [\"{}\"],\n",
        action.object_api_name
    ));
    output.push_str("    affectedLinkTypes: [],\n");
    output.push_str("    typeGroups: [],\n");
    output.push_str("  },\n");
    output.push_str("});\n");
    output
}

fn render_action_parameter(parameter: &ActionParameterDefinition) -> String {
    let mut output = String::new();
    output.push_str("    {\n");
    if parameter.id == "objectToModifyParameter" {
        output.push_str("      id: MODIFY_OBJECT_PARAMETER,\n");
    } else {
        output.push_str(&format!("      id: \"{}\",\n", parameter.id));
    }
    output.push_str(&format!(
        "      displayName: \"{}\",\n",
        parameter.display_name
    ));
    match &parameter.parameter_type {
        ActionParameterTypeDefinition::Primitive(name) => {
            output.push_str(&format!("      type: \"{}\",\n", name));
            output.push_str(&format!(
                "      validation: {{ required: {} }},\n",
                parameter.required
            ));
        }
        ActionParameterTypeDefinition::ObjectReference { object_api_name } => {
            output.push_str("      type: {\n");
            output.push_str("        type: \"objectReference\",\n");
            output.push_str("        objectReference: {\n");
            output.push_str(&format!(
                "          objectTypeId: \"{}\",\n",
                object_api_name
            ));
            output.push_str("        },\n");
            output.push_str("      },\n");
            output.push_str(&format!(
                "      validation: {{ allowedValues: {{ type: \"objectQuery\" }}, required: {} }},\n",
                parameter.required
            ));
        }
    }
    output.push_str("    },\n");
    output
}

fn render_delete_target_parameter(object_api_name: &str) -> String {
    let mut output = String::new();
    output.push_str("    {\n");
    output.push_str("      id: DELETE_OBJECT_PARAMETER,\n");
    output.push_str("      displayName: \"Delete Object\",\n");
    output.push_str("      type: {\n");
    output.push_str("        type: \"objectReference\",\n");
    output.push_str("        objectReference: {\n");
    output.push_str(&format!(
        "          objectTypeId: \"{}\",\n",
        object_api_name
    ));
    output.push_str("        },\n");
    output.push_str("      },\n");
    output.push_str(
        "      validation: { allowedValues: { type: \"objectQuery\" }, required: true },\n",
    );
    output.push_str("    },\n");
    output
}

fn render_action_rule(action: &ActionDefinition) -> String {
    match &action.kind {
        ActionKind::Create {
            property_mappings, ..
        } => render_create_rule(action, property_mappings),
        ActionKind::Modify {
            property_mappings, ..
        } => render_modify_rule(property_mappings),
        ActionKind::Delete => render_delete_rule(),
    }
}

fn render_create_rule(
    action: &ActionDefinition,
    property_mappings: &[crate::ontology::ActionPropertyMapping],
) -> String {
    let mut output = String::new();
    output.push_str("    {\n");
    output.push_str("      type: \"addObjectRule\",\n");
    output.push_str("      addObjectRule: {\n");
    output.push_str(&format!(
        "        objectTypeId: \"{}\",\n",
        action.object_api_name
    ));
    output.push_str("        propertyValues: {\n");
    for mapping in property_mappings {
        output.push_str(&format!(
            "          {}: {{ type: \"parameterId\", parameterId: \"{}\" }},\n",
            mapping.property_api_name, mapping.parameter_id
        ));
    }
    output.push_str("        },\n");
    output.push_str("        structFieldValues: {},\n");
    output.push_str("      },\n");
    output.push_str("    },\n");
    output
}

fn render_modify_rule(property_mappings: &[crate::ontology::ActionPropertyMapping]) -> String {
    let mut output = String::new();
    output.push_str("    {\n");
    output.push_str("      type: \"modifyObjectRule\",\n");
    output.push_str("      modifyObjectRule: {\n");
    output.push_str("        objectToModify: MODIFY_OBJECT_PARAMETER,\n");
    output.push_str("        propertyValues: {\n");
    for mapping in property_mappings {
        output.push_str(&format!(
            "          {}: {{ type: \"parameterId\", parameterId: \"{}\" }},\n",
            mapping.property_api_name, mapping.parameter_id
        ));
    }
    output.push_str("        },\n");
    output.push_str("        structFieldValues: {},\n");
    output.push_str("      },\n");
    output.push_str("    },\n");
    output
}

fn render_delete_rule() -> String {
    let mut output = String::new();
    output.push_str("    {\n");
    output.push_str("      type: \"deleteObjectRule\",\n");
    output.push_str("      deleteObjectRule: {\n");
    output.push_str("        objectToDelete: DELETE_OBJECT_PARAMETER,\n");
    output.push_str("      },\n");
    output.push_str("    },\n");
    output
}
