//! Compile the Federate Protocol messages (proto/README.md) without protoc.

fn main() {
    let files = ["proto/datatypes.proto", "proto/RTIambassador.proto", "proto/FederateAmbassador.proto"];
    for f in files {
        println!("cargo:rerun-if-changed={f}");
    }
    let fds = protox::compile(files, ["proto"]).expect("compiling the Federate Protocol protos");
    prost_build::Config::new().compile_fds(fds).expect("generating Rust from the protos");
}
