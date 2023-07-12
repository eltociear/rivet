/*
 * Rivet API
 *
 * No description provided (generated by Openapi Generator https://github.com/openapitools/openapi-generator)
 *
 * The version of the OpenAPI document: 0.0.1
 * 
 * Generated by: https://openapi-generator.tech
 */

/// CloudVersionMatchmakerGameModeFindConfig : Configures the requirements and authentication for the `find` endpoint.



#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct CloudVersionMatchmakerGameModeFindConfig {
    #[serde(rename = "identity_requirement")]
    pub identity_requirement: crate::models::CloudVersionMatchmakerGameModeIdentityRequirement,
    #[serde(rename = "verification_config", skip_serializing_if = "Option::is_none")]
    pub verification_config: Option<Box<crate::models::CloudVersionMatchmakerGameModeVerificationConfig>>,
}

impl CloudVersionMatchmakerGameModeFindConfig {
    /// Configures the requirements and authentication for the `find` endpoint.
    pub fn new(identity_requirement: crate::models::CloudVersionMatchmakerGameModeIdentityRequirement) -> CloudVersionMatchmakerGameModeFindConfig {
        CloudVersionMatchmakerGameModeFindConfig {
            identity_requirement,
            verification_config: None,
        }
    }
}

