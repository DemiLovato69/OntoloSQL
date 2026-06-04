#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleDefinition {
    pub value_types: Vec<ValueTypeDefinition>,
    pub objects: Vec<ObjectDefinition>,
    pub links: Vec<LinkDefinition>,
    pub actions: Vec<ActionDefinition>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueTypeDefinition {
    pub const_name: String,
    pub api_name: String,
    pub display_name: String,
    pub values: Vec<String>,
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
    pub value_type_const_name: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionDefinition {
    pub const_name: String,
    pub api_name: String,
    pub display_name: String,
    pub object_const_name: String,
    pub object_api_name: String,
    pub kind: ActionKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionKind {
    Create {
        parameters: Vec<ActionParameterDefinition>,
        property_mappings: Vec<ActionPropertyMapping>,
    },
    Modify {
        parameters: Vec<ActionParameterDefinition>,
        property_mappings: Vec<ActionPropertyMapping>,
    },
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionParameterDefinition {
    pub id: String,
    pub display_name: String,
    pub parameter_type: ActionParameterTypeDefinition,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionParameterTypeDefinition {
    Primitive(String),
    ObjectReference { object_api_name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionPropertyMapping {
    pub property_api_name: String,
    pub parameter_id: String,
}
