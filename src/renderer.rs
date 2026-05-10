use crate::ontology::{
    LinkDefinition, LinkEndpointDefinition, ModuleDefinition, ObjectDefinition, PropertyDefinition,
};

pub fn render_module(module: &ModuleDefinition) -> String {
    let import = if module.links.is_empty() {
        "import { defineObject } from \"@osdk/maker\";\n\n"
    } else {
        "import { defineLink, defineObject } from \"@osdk/maker\";\n\n"
    };

    let mut output = String::from(import);

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
    format!(
        "    \"{}\": {{ type: \"{}\", displayName: \"{}\" }},\n",
        property.api_name, property.osdk_type, property.display_name
    )
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
