use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};
#[derive(OpenApi)]
#[openapi(info(title="AnchorFS API",version="1.0.0-stage0"),components(schemas(crate::dto::ErrorCodeDto,crate::dto::ErrorBody,crate::dto::InodeAttributes,crate::dto::DirectoryPage,crate::dto::OperationStatus)),modifiers(&CredentialSecurity))]
pub struct ApiDoc;
struct CredentialSecurity;
impl Modify for CredentialSecurity {
    fn modify(&self, doc: &mut utoipa::openapi::OpenApi) {
        doc.components
            .as_mut()
            .expect("components")
            .add_security_scheme(
                "credential",
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .description(Some(
                            "administrator-issued opaque credential scoped to workspace and role",
                        ))
                        .build(),
                ),
            );
    }
}
pub fn document() -> utoipa::openapi::OpenApi {
    let (_, mut doc) =
        crate::app::contract_router(&crate::config::Config::contract_defaults()).split_for_parts();
    let root = ApiDoc::openapi();
    doc.info = root.info.clone();
    doc.merge(root);
    doc.info.version = anchorfs_core::API_DOCUMENT_VERSION.to_owned();
    doc
}
