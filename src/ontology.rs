#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleDefinition {
    pub objects: Vec<ObjectDefinition>,
    pub links: Vec<LinkDefinition>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectDefinition {
    pub const_name: String,
    pub api_name: String,
    pub display_name: String,
    pub plural_display_name: String,
    pub title_property_api_name: String,
    pub primary_key_property_api_name: String,
    pub properties: Vec<PropertyDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyDefinition {
    pub api_name: String,
    pub display_name: String,
    pub osdk_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkDefinition {
    pub const_name: String,
    pub api_name: String,
    pub one: LinkEndpointDefinition,
    pub to_many: LinkEndpointDefinition,
    pub many_foreign_key_property: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkEndpointDefinition {
    pub object_const_name: String,
    pub api_name: String,
    pub display_name: String,
    pub plural_display_name: String,
}
