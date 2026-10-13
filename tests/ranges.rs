use repo_doctor::checks::runtime::Requirement;

#[test]
fn npm_compatibility_cases() {
    for (range, version, expected) in [
        ("22.1.0", "22.1.0", true),
        ("22.1.0", "22.1.1", false),
        ("22", "22.9.0", true),
        ("22", "23.0.0", false),
        (">=22 <23", "22.1.0", true),
        ("^22.1.0", "22.5.0", true),
        ("^22.1.0", "23.0.0", false),
        ("20 || 22", "22.1.0", true),
        ("20 || 22", "21.0.0", false),
        (">=22 <23", "22.1.0-rc.1", false),
        (">=22.1.0-rc.1 <23", "22.1.0-rc.2", true),
        ("~22.1.0", "22.2.0", false),
    ] {
        assert_eq!(
            Requirement::parse("node", range, "package.json", false)
                .unwrap()
                .matches(version),
            expected,
            "{range}: {version}"
        );
    }
    for (file, version, expected) in [
        ("22", "22.9.0", true),
        ("22.1", "22.1.5", true),
        ("22.1", "22.2.0", false),
        ("v22.1.0", "22.1.0", true),
        ("v22.1.0", "22.1.1", false),
    ] {
        assert_eq!(
            Requirement::parse("node", file, ".nvmrc", true)
                .unwrap()
                .matches(version),
            expected
        );
    }
    assert!(Requirement::parse("node", "lts/*", ".nvmrc", true).is_err());
}

#[test]
fn python_specifiers_and_numeric_files() {
    for (range, version, expected) in [
        (">=3.11,<3.14", "3.12.2", true),
        (">=3.11,<3.14", "3.14.0", false),
        (">=3.11,!=3.12.2", "3.12.2", false),
        ("~=3.11.0", "3.11.8", true),
        ("~=3.11.0", "3.12.0", false),
        ("==3.12.*", "3.12.8", true),
    ] {
        assert_eq!(
            Requirement::parse("python", range, "pyproject.toml", false)
                .unwrap()
                .matches(version),
            expected
        );
    }
    for (value, version, expected) in [
        ("3.12", "3.12.9", true),
        ("3.12", "3.13.0", false),
        ("3.12.2", "3.12.3", false),
    ] {
        assert_eq!(
            Requirement::parse("python", value, ".python-version", true)
                .unwrap()
                .matches(version),
            expected
        );
    }
    assert!(Requirement::parse("python", "3.11\n3.12", ".python-version", true).is_err());
}
