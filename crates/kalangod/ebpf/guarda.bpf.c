// KalangoOS · guarda do kalangod (roda dentro do kernel, carregado pelo kalangod em Rust).
//
// 1. Regra inviolável: processos de agentes (descendentes do cgroup dos agentes) não abrem nada dentro de uma
//    pasta protegida — nem como root, nem no nível total.
// 2. Auditoria: cada arquivo aberto e cada programa executado por um agente vira um evento, independente do que
//    o agente relata. O kalangod grava os eventos na linha do tempo.
//
// Escrito em C porque usa CO-RE (relocações de campos do kernel por BTF), que o eBPF em Rust ainda não oferece:
// o mesmo objeto funciona em kernels diferentes sem recompilar.
#include <linux/types.h>
#include <linux/bpf.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_tracing.h>
#include <bpf/bpf_core_read.h>

#define EPERM 1
#define MAX_DEPTH 24
#define NAME_LEN 64
#define MAX_AGENTES 4096

// Só os campos usados; CO-RE acha a posição real no kernel em execução.
struct super_block { __u32 s_dev; } __attribute__((preserve_access_index));
struct inode { unsigned long i_ino; struct super_block *i_sb; } __attribute__((preserve_access_index));
struct qstr { const unsigned char *name; } __attribute__((preserve_access_index));
struct dentry { struct dentry *d_parent; struct inode *d_inode; struct qstr d_name; } __attribute__((preserve_access_index));
struct path { void *mnt; struct dentry *dentry; } __attribute__((preserve_access_index));
struct file { struct path f_path; unsigned int f_flags; } __attribute__((preserve_access_index));
struct linux_binprm { struct file *file; const char *filename; } __attribute__((preserve_access_index));

struct chave_inode { __u64 dev; __u64 ino; };

struct evento {
    __u32 tipo;      // 1 = abriu arquivo, 2 = executou programa
    __u32 bloqueado; // 1 = negado pela regra
    __u32 pid;
    __u32 uid;
    __u64 cgroup;
    __u64 dev;
    __u64 ino;
    __u32 flags;
    char nome[NAME_LEN];
};

// configuração escrita pelo kalangod: [2] = processo do kalangod que está descobrindo as chaves (dispositivo, inode) das pastas protegidas
// ([0] e [1], o cgroup dos agentes e o nível dele, saíram: os cgroups dos agentes estão em CG_AGENTES)
struct { __uint(type, BPF_MAP_TYPE_ARRAY); __uint(max_entries, 3); __type(key, __u32); __type(value, __u64); } CONFIG SEC(".maps");
// os cgroups dos agentes: o do sistema (/kalango.slice/kalango-agentes.slice) e o de cada gerenciador de usuário do systemd
// (user.slice/user-UID.slice/user@UID.service/kalango.slice/kalango-agentes.slice), onde roda o kalango-fluxosd e o que
// `systemd-run --user` cria. id do cgroup -> 1; o kalangod os registra e tira (varredura de 1 s). Sem limite por usuário.
struct { __uint(type, BPF_MAP_TYPE_HASH); __uint(max_entries, MAX_AGENTES); __type(key, __u64); __type(value, __u8); } CG_AGENTES SEC(".maps");
// pastas protegidas, por (dispositivo, inode)
struct { __uint(type, BPF_MAP_TYPE_HASH); __uint(max_entries, 4096); __type(key, struct chave_inode); __type(value, __u8); } PROTEGIDAS SEC(".maps");
struct { __uint(type, BPF_MAP_TYPE_RINGBUF); __uint(max_entries, 1 << 20); } EVENTOS SEC(".maps");

// um nível dos ancestrais do processo: o cgroup dele nesse nível é um cgroup de agentes? (cada passo tem o nível constante:
// sem laço, o verificador do kernel não recusa o programa — com o contador na pilha ele dizia "infinite loop detected" e a
// guarda inteira ficava desligada). O cgroup dos agentes fica no nível 2 (sistema) ou 5 (gerenciador de usuário); os níveis
// 1 a 8 cobrem os dois com folga. Um nível além da profundidade do processo devolve id 0, que não está no mapa.
#define NIVEL(n) { __u64 cg = bpf_get_current_ancestor_cgroup_id(n); \
    if (cg && bpf_map_lookup_elem(&CG_AGENTES, &cg)) return 1; }

static __always_inline int eh_agente(void) {
    NIVEL(1) NIVEL(2) NIVEL(3) NIVEL(4) NIVEL(5) NIVEL(6) NIVEL(7) NIVEL(8)
    return 0;
}

static __always_inline void emitir(__u32 tipo, __u32 bloqueado, __u64 dev, __u64 ino, __u32 flags, const unsigned char *nome) {
    struct evento *e = bpf_ringbuf_reserve(&EVENTOS, sizeof(*e), 0);
    if (!e)
        return;
    e->tipo = tipo;
    e->bloqueado = bloqueado;
    e->pid = bpf_get_current_pid_tgid() >> 32;
    e->uid = (__u32)bpf_get_current_uid_gid();
    e->cgroup = bpf_get_current_cgroup_id();
    e->dev = dev;
    e->ino = ino;
    e->flags = flags;
    bpf_probe_read_kernel_str(e->nome, sizeof(e->nome), nome);
    bpf_ringbuf_submit(e, 0);
}

#define O_ESCRITA 3 /* O_WRONLY | O_RDWR */

SEC("lsm/file_open")
int BPF_PROG(guarda_abrir, struct file *f) {
    struct dentry *d = BPF_CORE_READ(f, f_path.dentry);
    __u32 k2 = 2;
    __u64 *sonda = bpf_map_lookup_elem(&CONFIG, &k2);
    if (sonda && *sonda && (bpf_get_current_pid_tgid() >> 32) == *sonda) {
        // descoberta: o kalangod abriu uma pasta protegida e quer saber como o kernel a identifica
        // (em btrfs e overlay o st_dev visto pelo stat() não é o dispositivo do superbloco)
        struct inode *in = BPF_CORE_READ(d, d_inode);
        emitir(3, 0, BPF_CORE_READ(in, i_sb, s_dev), BPF_CORE_READ(in, i_ino), 0, BPF_CORE_READ(d, d_name.name));
        return 0;
    }
    if (!eh_agente())
        return 0;
    struct inode *alvo = BPF_CORE_READ(d, d_inode);
    __u64 dev = BPF_CORE_READ(alvo, i_sb, s_dev);
    __u64 ino = BPF_CORE_READ(alvo, i_ino);
    const unsigned char *nome = BPF_CORE_READ(d, d_name.name);
    __u32 flags = BPF_CORE_READ(f, f_flags);
    // sobe pelos diretórios procurando uma pasta protegida
    struct dentry *cur = d;
    for (int i = 0; i < MAX_DEPTH; i++) {
        struct inode *in = BPF_CORE_READ(cur, d_inode);
        if (in) {
            struct chave_inode k = { .dev = BPF_CORE_READ(in, i_sb, s_dev), .ino = BPF_CORE_READ(in, i_ino) };
            if (bpf_map_lookup_elem(&PROTEGIDAS, &k)) {
                emitir(1, 1, dev, ino, flags, nome);
                return -EPERM;
            }
        }
        struct dentry *pai = BPF_CORE_READ(cur, d_parent);
        if (!pai || pai == cur)
            break;
        cur = pai;
    }
    // leituras são frequentes demais para auditar uma a uma; escritas sempre viram evento
    if (flags & O_ESCRITA)
        emitir(1, 0, dev, ino, flags, nome);
    return 0;
}

SEC("lsm/bprm_check_security")
int BPF_PROG(guarda_executar, struct linux_binprm *b) {
    if (!eh_agente())
        return 0;
    emitir(2, 0, 0, 0, 0, (const unsigned char *)BPF_CORE_READ(b, filename));
    return 0;
}

char LICENSE[] SEC("license") = "GPL";
