use std::{env, fs, path::PathBuf};

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed=build.rs");

    // Write linker script
    let mut linker_script = out_dir.clone();
    linker_script.push("linker.ld");
    let base_address = env::var("BASE_ADDRESS").unwrap_or_else(|_| "0x80400000".to_string());
    fs::write(
        &linker_script,
        format!(
            "OUTPUT_ARCH(riscv)\nENTRY(_start)\nBASE_ADDRESS = {};\n\n\
            SECTIONS\n{{\n    . = BASE_ADDRESS;\n    \
            .text : {{\n        *(.text.entry)\n        *(.text .text.*)\n    }}\n    \
            .rodata : {{\n        *(.rodata .rodata.*)\n        *(.srodata .srodata.*)\n    }}\n    \
            .data : {{\n        *(.data .data.*)\n        *(.sdata .sdata.*)\n    }}\n    \
            .bss : {{\n        *(.bss .bss.*)\n        *(.sbss .sbss.*)\n    }}\n    \
            /DISCARD/ : {{\n        *(.eh_frame)\n    }}\n}}\n",
            base_address
        ),
    )
    .unwrap();
    println!("cargo:rustc-link-search={}", out_dir.display());
    println!("cargo:rustc-link-arg=-Tlinker.ld");

    // Compile doomgeneric if we are building the doom binary
    let doom_src = PathBuf::from("src/bin/doomgeneric_src/doomgeneric");
    let doom_include = PathBuf::from("src/bin/doomgeneric_src/include");
    println!("cargo:rerun-if-changed={}", doom_src.display());
    println!("cargo:rerun-if-changed={}", doom_include.display());
    if doom_src.exists() {
        let mut build = cc::Build::new();
        build.target("riscv64gc-unknown-none-elf")
             .flag("-march=rv64gc")
             .flag("-mabi=lp64d")
             .flag("-O3")
             .flag("-ffreestanding")
             .flag("-fno-builtin")
             .flag("-nostdlib")
             .flag("-mcmodel=medany")
             .flag("-Wno-unused-parameter")
             .flag("-Wno-sign-compare")
             .flag("-Wno-implicit-function-declaration")
             .include(&doom_src)
             .include(&doom_include);

        let files = fs::read_dir(&doom_src).unwrap();
        for file in files {
            let path = file.unwrap().path();
            if path.extension().and_then(|s| s.to_str()) == Some("c") {
                println!("cargo:rerun-if-changed={}", path.display());
                let filename = path.file_name().unwrap().to_str().unwrap();
                // Skip platform-specific doomgeneric frontends (SDL, X11, allegro, etc.)
                if filename.starts_with("doomgeneric_") && filename != "doomgeneric.c" {
                    continue;
                }
                // Skip Allegro sound/music
                if filename.contains("allegro") {
                    continue;
                }
                // Skip SDL sound/music
                if filename.contains("sdl") || filename.contains("SDL") {
                    continue;
                }
                // Skip Linux VT / X11 specific
                if filename.contains("linuxvt") || filename.contains("xlib") {
                    continue;
                }
                // Skip emscripten / soso / SOSOX
                if filename.contains("emscripten") || filename.contains("soso") {
                    continue;
                }
                // Skip network code
                if filename.starts_with("net_") || filename.starts_with("i_net") {
                    continue;
                }
                // Skip DOS endoom
                if filename.contains("endoom") {
                    continue;
                }
                build.file(path);
            }
        }
        build.compile("doomgeneric");
    }
}
