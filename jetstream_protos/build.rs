use std::{env, fs, path::Path};

use anyhow::Result;
use tonic_build::manual::{Builder, Method, Service};

fn main() -> Result<()> {
    std::env::set_var("PROTOC", protobuf_src::protoc());

    // build protos
    tonic_build::configure().compile_protos(&["protos/jetstream.proto"], &["protos"])?;

    // build protos without tonic (wasm)
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not found");
    let out_dir_path = Path::new(&out_dir).join("no-tonic");
    fs::create_dir_all(&out_dir_path).expect("failed to create out no-tonic directory");
    tonic_build::configure()
        .build_client(false)
        .build_server(false)
        .out_dir(out_dir_path)
        .compile_protos(&["protos/jetstream.proto"], &["protos"])?;

    // build with custom struct configuration
    let jetstream_service = Service::builder()
        .name("Jetstream")
        .package("jetstream")
        .method(
            Method::builder()
                .name("subscribe")
                .route_name("Subscribe")
                .input_type("crate::jetstream::SubscribeRequest")
                .output_type("crate::jetstream::SubscribeUpdate")
                .codec_path("tonic::codec::ProstCodec")
                .client_streaming()
                .server_streaming()
                .build(),
        )
        .method(
            Method::builder()
                .name("ping")
                .route_name("Ping")
                .input_type("crate::jetstream::PingRequest")
                .output_type("crate::jetstream::PongResponse")
                .codec_path("tonic::codec::ProstCodec")
                .build(),
        )
        .method(
            Method::builder()
                .name("get_version")
                .route_name("GetVersion")
                .input_type("crate::jetstream::GetVersionRequest")
                .output_type("crate::jetstream::GetVersionResponse")
                .codec_path("tonic::codec::ProstCodec")
                .build(),
        )
        .method(
            Method::builder()
                .name("get_slot")
                .route_name("GetSlot")
                .input_type("crate::jetstream::GetSlotRequest")
                .output_type("crate::jetstream::GetSlotResponse")
                .codec_path("tonic::codec::ProstCodec")
                .build(),
        )
        .build();

    Builder::new()
        .build_client(false)
        .compile(&[jetstream_service]);

    Ok(())
}
