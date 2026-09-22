use std::process::Command;

fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let obj_path = format!("{}/boot.o", out_dir);

    let status = Command::new("nasm")
        .args(["-f", "elf64", "boot/boot.asm", "-o", &obj_path])
        .status()
        .expect("nasm no encontrado — instala nasm (pacman -S nasm)");

    if !status.success() {
        panic!("nasm falló al ensamblar boot.asm");
    }

    println!("cargo:rustc-link-arg={}", obj_path);
    println!("cargo:rerun-if-changed=boot/boot.asm");
    println!("cargo:rerun-if-changed=../targets/linker.ld");
}
