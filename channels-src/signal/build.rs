fn main() {
    prost_build::Config::new()
        .compile_protos(
            &[
                "protobuf/SignalService.proto",
                "protobuf/WebSocketResources.proto",
                "protobuf/Provisioning.proto",
            ],
            &["protobuf/"],
        )
        .expect("Failed to compile Signal protobuf definitions");
}
