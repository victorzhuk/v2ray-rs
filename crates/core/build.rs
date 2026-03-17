fn main() {
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());

    prost_build::Config::new()
        .out_dir(&out_dir)
        .compile_protos(&["proto/v2ray_geoip.proto"], &["."])
        .expect("failed to compile v2ray_geoip.proto");

    prost_build::Config::new()
        .out_dir(&out_dir)
        .compile_protos(&["proto/v2ray_geosite.proto"], &["."])
        .expect("failed to compile v2ray_geosite.proto");
}
