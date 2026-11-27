use super::admin::{agent_landlock, system_read_roots};
use crate::policy::is_within;
use std::path::PathBuf;

fn reaches(l: &crate::limits::Landlock, codes: &std::path::Path) -> Vec<String> {
    l.leitura.iter().chain(&l.escrita).filter(|g| is_within(codes, g) || is_within(g, codes)).map(|g| g.display().to_string()).collect()
}

/// The one assembly of the rules of a command and of a connector cuts the system's read grants around the Mind's
/// confirmation codes: no grant is the folder, holds it or lies inside it (`/run` is not granted whole, only
/// listable), and the rest of the system stays granted. No host needed: the folder is a place, not a file.
#[test]
fn the_landlock_assembly_leaves_out_the_mind_confirmation_codes() {
    let codes = [PathBuf::from("/run/user/4242/kalango/mind-confirm"), PathBuf::from("/run/user/1000/kalango/mind-confirm")];
    let l = agent_landlock(&codes, vec![PathBuf::from("/srv/trabalho")], vec![], vec![PathBuf::from("/dev/null")], None, vec![], vec![]);
    for c in &codes {
        assert!(reaches(&l, c).is_empty(), "{:?} reaches {}", reaches(&l, c), c.display());
    }
    assert!(!l.leitura.contains(&PathBuf::from("/run")) && l.listar.contains(&PathBuf::from("/run")), "{:?}", l.listar);
    for whole in ["/usr", "/etc", "/proc", "/sys", "/dev", "/var/lib"] {
        assert!(l.leitura.contains(&PathBuf::from(whole)), "{whole} stays granted");
    }
    assert!(l.leitura.contains(&PathBuf::from("/srv/trabalho")) && l.escrita == vec![PathBuf::from("/dev/null")]);
    // with nothing protected inside the system folders, they are granted exactly as listed
    let l = agent_landlock(&[PathBuf::from("/home/ana/.ssh")], vec![], vec![], vec![], None, vec![], vec![]);
    assert_eq!(l.leitura, system_read_roots());
    assert!(l.listar.is_empty());
}

/// The two callers (a command in `admin::task_run`, a connector in `conector::spawn`) get their kernel rules only from
/// `agent_landlock`: no `Landlock { .. }` of their own and no direct use of the system's list, so putting either back
/// to the whole of `/run` is a test failure, with no host needed. (A source check: the behaviour of the assembly is in
/// the test above, and with the real `/run/user/UID` in the host tests.)
#[test]
fn a_command_and_a_connector_build_their_rules_only_through_the_assembly() {
    let admin = include_str!("admin.rs");
    let task = &admin[admin.find("fn task_run(").unwrap()..];
    let task = &task[..task.find("\nfn ").unwrap_or(task.len())];
    let conector = include_str!("../conector.rs");
    let spawn = &conector[conector.find("pub fn spawn(").unwrap()..];
    let spawn = &spawn[..spawn.find("\npub fn ").or(spawn.find("\nfn ")).unwrap_or(spawn.len())];
    for (who, code) in [("task_run", task), ("conector::spawn", spawn)] {
        assert!(code.contains("agent_landlock("), "{who} must build its rules with agent_landlock");
        assert!(!code.contains("Landlock {") && !code.contains("SISTEMA_LEITURA"), "{who} builds a Landlock of its own");
    }
}
