// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::{Path, PathBuf};

const DEFINES: &[(&str, Option<&str>)] = &[
    ("Z80_STATIC", None),
    ("Z80_WITH_LOCAL_HEADER", None),
    ("Z80_WITH_Q", None),
    ("Z80_WITH_ZILOG_NMOS_LD_A_IR_BUG", None),
    ("Z80_WITH_PRECOMPUTED_DAA", None),
    ("Z80_WITH_VOLATILE_CYCLES", None),
    ("ZX_TESTING", None),
    ("Z80_EXTERNAL_HEADER", Some("\"z_types.h\"")),
    ("Z80_INSN_CLOCK_EXTERNAL_HEADER", None),
    ("Z80_INSN_CLOCK_STATIC", None),
    ("Z80_INSN_CLOCK_WITH_LOCAL_HEADER", None),
    ("Z80_INSN_CLOCK", Some("2")),
];

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let core_zx = root.join("core/zx");
    let core_sound = root.join("core/sound");
    let app = root.join("app");
    let includes = [
        root.join("core/z80"),
        core_zx.clone(),
        core_sound.clone(),
        app.clone(),
        root.join("third_party/Z80/API"),
        root.join("third_party/Z80InsnClock/API"),
    ];

    let c_files = |dir: &Path| -> Vec<PathBuf> {
        let mut v: Vec<_> = std::fs::read_dir(dir)
            .expect("read source dir")
            .map(|e| e.expect("dir entry").path())
            .filter(|p| p.extension().is_some_and(|e| e == "c"))
            .collect();
        v.sort();
        v
    };

    // ZX_SANITIZE=1 builds the C with AddressSanitizer and UBSan (host only).
    println!("cargo:rerun-if-env-changed=ZX_SANITIZE");
    let sanitize = std::env::var("ZX_SANITIZE").is_ok_and(|v| v == "1");
    if sanitize {
        // The runtime must be linked into the final (Rust) binary, and
        // rustc-link-arg of a library crate does not reach its dependents, so
        // link the sanitizer runtime as a library. macOS: the dynamic runtime
        // in clang's resource dir also carries UBSan.
        let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
        let dir = std::process::Command::new(&cc)
            .arg("-print-resource-dir")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .expect("ZX_SANITIZE=1 needs a clang that supports -print-resource-dir");
        // rustc adds OUT_DIR to the run-time search path of the binaries it
        // links, so a copy of the runtime there is found when they start.
        let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
        let rt = "libclang_rt.asan_osx_dynamic.dylib";
        std::fs::copy(format!("{dir}/lib/darwin/{rt}"), format!("{out_dir}/{rt}")).expect("copy the ASan runtime");
        println!("cargo:rustc-link-search=native={out_dir}");
        println!("cargo:rustc-link-lib=dylib=clang_rt.asan_osx_dynamic");
    }

    let build = |files: Vec<PathBuf>, name: &str, strict: bool| {
        let mut b = cc::Build::new();
        b.files(&files).std("c11").warnings(false);
        for inc in &includes {
            b.include(inc);
        }
        for (k, v) in DEFINES {
            b.define(k, *v);
        }
        if sanitize {
            b.flag("-fsanitize=address,undefined").flag("-fno-omit-frame-pointer");
        }
        if strict {
            b.flag("-Wall").flag("-Wextra").flag("-Werror");
        }
        for f in &files {
            println!("cargo:rerun-if-changed={}", f.display());
        }
        b.compile(name);
    };

    // Our code is held to -Werror; the vendored libraries are built as they are. core/sound
    // includes engines_test.c (the test fixtures' engines), which the firmware build leaves out.
    build(c_files(&core_zx), "zxcore", true);
    build(c_files(&core_sound), "zxsound", true);
    build(c_files(&app), "fmzxapp", true);
    build(
        vec![
            root.join("third_party/Z80/sources/Z80.c"),
            root.join("third_party/Z80InsnClock/sources/Z80InsnClock.c"),
        ],
        "z80vendor",
        false,
    );

    for dir in [
        core_zx.clone(),
        core_sound.clone(),
        app.clone(),
        root.join("core/z80"),
        root.join("third_party/Z80/API"),
        root.join("third_party/Z80InsnClock/API"),
    ] {
        println!("cargo:rerun-if-changed={}", dir.display());
    }

    let mut gen = bindgen::Builder::default()
        .header(core_zx.join("zx.h").to_string_lossy())
        .header(core_sound.join("sound.h").to_string_lossy())
        .header(app.join("pixels.h").to_string_lossy())
        .header(app.join("lcdsched.h").to_string_lossy())
        .header(app.join("keymap.h").to_string_lossy())
        .header(app.join("lz4.h").to_string_lossy())
        .header(app.join("pack.h").to_string_lossy())
        .header(app.join("ui.h").to_string_lossy())
        .header(app.join("menu.h").to_string_lossy())
        .allowlist_function("(zx|pix|lcd|km|pack|lz4|ui|menu|snd)_.*")
        .allowlist_type("(zx|pix|lcd|km|pack|snd)_.*")
        .allowlist_type("keymap")
        .allowlist_type("menu")
        .allowlist_var("(ZX|LCD|PIX|KM|PACK|UI|EV|VIEW|CMD|SND|snd)_.*")
        .derive_default(false)
        .layout_tests(false);
    for inc in &includes {
        gen = gen.clang_arg(format!("-I{}", inc.display()));
    }
    for (k, v) in DEFINES {
        gen = gen.clang_arg(match v {
            Some(v) => format!("-D{k}={v}"),
            None => format!("-D{k}"),
        });
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    gen.generate()
        .expect("bindgen failed")
        .write_to_file(out.join("bindings.rs"))
        .expect("write bindings");
}
