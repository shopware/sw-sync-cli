use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let profiles_out_dir = Path::new(&out_dir).join("profiles");

    fs::create_dir_all(&profiles_out_dir).unwrap();

    let files = [
        "src/resources/profiles/manufacturer.yaml",
        "src/resources/profiles/product_with_manufacturer.yaml",
        "src/resources/profiles/product_required.yaml",
        "src/resources/profiles/product_variants.yaml",
    ];

    let mut profiles_content = String::new();

    for file in files.iter() {
        let file_name = Path::new(file).file_name().unwrap().to_str().unwrap();
        let dest_path = profiles_out_dir.join(file_name);

        fs::copy(file, &dest_path).unwrap();

        profiles_content.push_str(&format!(
            "    (\"{}\", include_bytes!(concat!(env!(\"OUT_DIR\"), \"/profiles/{}\"))),\n",
            file_name, file_name
        ));
    }

    let profiles_rs_content = format!(
        "pub const PROFILES: &[(&str, &[u8])] = &[\n{}];\n",
        profiles_content
    );

    let profiles_rs_path = Path::new(&out_dir).join("profiles.rs");
    fs::write(profiles_rs_path, profiles_rs_content).unwrap();
}
