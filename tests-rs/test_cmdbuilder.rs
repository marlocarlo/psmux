use super::*;

#[cfg(unix)]
#[test]
fn test_cwd_relative() {
    assert!(is_cwd_relative_path("."));
    assert!(is_cwd_relative_path("./foo"));
    assert!(is_cwd_relative_path("../foo"));
    assert!(!is_cwd_relative_path("foo"));
    assert!(!is_cwd_relative_path("/foo"));
}

#[test]
fn test_env() {
    let mut cmd = CommandBuilder::new("dummy");
    let package_authors = cmd.get_env("CARGO_PKG_AUTHORS");
    println!("package_authors: {:?}", package_authors);
    assert!(package_authors == Some(OsStr::new("Wez Furlong")));

    cmd.env("foo key", "foo value");
    cmd.env("bar key", "bar value");

    let iterated_envs = cmd.iter_extra_env_as_str().collect::<Vec<_>>();
    println!("iterated_envs: {:?}", iterated_envs);
    assert!(iterated_envs == vec![("bar key", "bar value"), ("foo key", "foo value")]);

    {
        let mut cmd = cmd.clone();
        cmd.env_remove("foo key");

        let iterated_envs = cmd.iter_extra_env_as_str().collect::<Vec<_>>();
        println!("iterated_envs: {:?}", iterated_envs);
        assert!(iterated_envs == vec![("bar key", "bar value")]);
    }

    {
        let mut cmd = cmd.clone();
        cmd.env_remove("bar key");

        let iterated_envs = cmd.iter_extra_env_as_str().collect::<Vec<_>>();
        println!("iterated_envs: {:?}", iterated_envs);
        assert!(iterated_envs == vec![("foo key", "foo value")]);
    }

    {
        let mut cmd = cmd.clone();
        cmd.env_clear();

        let iterated_envs = cmd.iter_extra_env_as_str().collect::<Vec<_>>();
        println!("iterated_envs: {:?}", iterated_envs);
        assert!(iterated_envs.is_empty());
    }
}

#[cfg(windows)]
#[test]
fn test_env_case_insensitive_override() {
    let mut cmd = CommandBuilder::new("dummy");
    cmd.env("Cargo_Pkg_Authors", "Not Wez");
    assert!(cmd.get_env("cargo_pkg_authors") == Some(OsStr::new("Not Wez")));

    cmd.env_remove("cARGO_pKG_aUTHORS");
    assert!(cmd.get_env("CARGO_PKG_AUTHORS").is_none());
}
