use tonic_prost_build::configure;

fn main() {
    const PROTOC_ENVAR: &str = "PROTOC";
    if std::env::var(PROTOC_ENVAR).is_err() {
        #[cfg(not(windows))]
        std::env::set_var(PROTOC_ENVAR, protobuf_src::protoc());
    }

    configure()
        .extern_path(".shared", "crate::grpc::shared")
        .compile_protos(
            &["proto/shredstream.proto", "proto/shared.proto"],
            &["proto"],
        )
        .unwrap();
}
