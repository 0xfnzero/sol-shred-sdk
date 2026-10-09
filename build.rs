use tonic_prost_build::configure;

fn main() {
    // librocksdb-sys does not emit its C++ runtime link when using a prebuilt archive.
    println!("cargo:rerun-if-env-changed=ROCKSDB_LIB_DIR");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos")
        && std::env::var_os("ROCKSDB_LIB_DIR").is_some()
    {
        println!("cargo:rustc-link-lib=c++");
    }
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
