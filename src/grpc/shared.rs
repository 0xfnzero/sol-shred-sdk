#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Socket {
    #[prost(string, tag = "1")]
    pub ip: String,
    #[prost(uint32, tag = "2")]
    pub port: u32,
} 