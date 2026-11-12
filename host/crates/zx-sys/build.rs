// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::{Path, PathBuf};

const DEFINES: &[(&str, Option<&str>)] = &[
    ("Z80_STATIC", None),
    ("Z80_WITH_LOCAL_HEADER", None),
    ("Z80_WITH_Q", None),
    ("Z80_WITH_ZILOG_NMOS_LD_A_IR_BUG", None),
    ("Z80_WITH_PRECOMPUTED_DAA", None),
    ("Z80_EXTERNAL_HEADER", Some("\"z_types.h\"")),
    ("Z80_INSN_CLOCK_EXTERNAL_HEADER", None),
    ("Z80_INSN_CLOCK_STATIC", None),
    ("Z80_INSN_CLOCK_WITH_LOCAL_HEADER", None),
    ("Z80_INSN_CLOCK", Some("2")),
];

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let core_zx = root.join("core/zx");
    let includes = [
        root.join("core/z80"),
        core_zx.clone(),
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

    let build = |files: Vec<PathBuf>, name: &str, strict: bool| {
        let mut b = cc::Build::new();
        b.files(&files).std("c11").warnings(false);
        for inc in &includes {
            b.include(inc);
        }
        for (k, v) in DEFINES {
            b.define(k, *v);
        }
        if strict {
            b.flag("-Wall").flag("-Wextra").flag("-Werror");
        }
        for f in &files {
            println!("cargo:rerun-if-changed={}", f.display());
        }
        b.compile(name);
    };

    // Our code is held to -Werror; the vendored libraries are built as they are.
    build(c_files(&core_zx), "zxcore", true);
    build(
        vec![
            root.join("third_party/Z80/sources/Z80.c"),
            root.join("third_party/Z80InsnClock/sources/Z80InsnClock.c"),
        ],
        "z80vendor",
        false,
    );

    println!("cargo:rerun-if-changed={}", core_zx.join("zx.h").display());
    println!("cargo:rerun-if-changed={}", root.join("core/z80/z_types.h").display());

    let mut gen = bindgen::Builder::default()
        .header(core_zx.join("zx.h").to_string_lossy())
        .allowlist_function("zx_.*")
        .allowlist_type("zx_.*")
        .allowlist_var("ZX_.*")
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
