//! Wire shapes of the Numaflow CRDs, restricted to the fields `nfctl` reads.
//! Unknown fields are ignored so newer operators keep working.

pub mod isb;
pub mod monovertex;
pub mod pipeline;
pub mod pod;

use kube::core::{ApiResource, GroupVersionKind};

pub const GROUP: &str = "numaflow.numaproj.io";
pub const VERSION: &str = "v1alpha1";

#[must_use]
pub fn isb_resource() -> ApiResource {
    ApiResource::from_gvk_with_plural(
        &GroupVersionKind::gvk(GROUP, VERSION, "InterStepBufferService"),
        "interstepbufferservices",
    )
}

#[must_use]
pub fn monovertex_resource() -> ApiResource {
    ApiResource::from_gvk_with_plural(
        &GroupVersionKind::gvk(GROUP, VERSION, "MonoVertex"),
        "monovertices",
    )
}

#[must_use]
pub fn vertex_resource() -> ApiResource {
    ApiResource::from_gvk_with_plural(&GroupVersionKind::gvk(GROUP, VERSION, "Vertex"), "vertices")
}

#[must_use]
pub fn pipeline_resource() -> ApiResource {
    ApiResource::from_gvk_with_plural(
        &GroupVersionKind::gvk(GROUP, VERSION, "Pipeline"),
        "pipelines",
    )
}

/// Convert a Kubernetes timestamp into the domain's time type.
pub(crate) fn to_timestamp(
    t: &k8s_openapi::apimachinery::pkg::apis::meta::v1::Time,
) -> Option<nfctl_core::model::Timestamp> {
    nfctl_core::model::Timestamp::from_unix_nanos(t.0.as_nanosecond())
}
