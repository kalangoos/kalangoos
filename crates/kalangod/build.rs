//! Compila o programa eBPF (ebpf/guarda.bpf.c) com clang para o alvo BPF, com BTF para CO-RE.
use std::process::Command;

fn main() {
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("guarda.bpf.o");
    println!("cargo:rerun-if-changed=ebpf/guarda.bpf.c");
    let st = Command::new("clang")
        .args(["-O2", "-g", "-target", "bpf", "-D__TARGET_ARCH_x86", "-I/usr/include/x86_64-linux-gnu", "-c", "ebpf/guarda.bpf.c", "-o"])
        .arg(&out)
        .status()
        .expect("clang não encontrado: instale clang e libbpf-dev");
    assert!(st.success(), "falha ao compilar o programa eBPF");
}
