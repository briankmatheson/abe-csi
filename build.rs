
fn main() {
    tonic_build::configure()
        .build_server(true)
        .compile(&["proto/csi.proto"], &["proto"])
        .expect("Failed to compile CSI protos");
}
