fn main() {
    tonic_build::configure()
        .build_server(true)
        .compile_protos(&["proto/csi.proto"], &["proto"])
        .expect("Failed to compile CSI protos");
}
