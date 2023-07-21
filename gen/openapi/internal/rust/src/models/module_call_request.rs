/*
 * Rivet API
 *
 * No description provided (generated by Openapi Generator https://github.com/openapitools/openapi-generator)
 *
 * The version of the OpenAPI document: 0.0.1
 * 
 * Generated by: https://openapi-generator.tech
 */




#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct ModuleCallRequest {
    #[serde(rename = "namespace_id", skip_serializing_if = "Option::is_none")]
    pub namespace_id: Option<uuid::Uuid>,
    #[serde(rename = "parameters", deserialize_with = "Option::deserialize")]
    pub parameters: Option<serde_json::Value>,
}

impl ModuleCallRequest {
    pub fn new(parameters: Option<serde_json::Value>) -> ModuleCallRequest {
        ModuleCallRequest {
            namespace_id: None,
            parameters,
        }
    }
}


