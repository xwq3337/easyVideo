#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantChoice {
    pub uri: String,
    pub bandwidth: u64,
    pub resolution: Option<(u64, u64)>,
}
