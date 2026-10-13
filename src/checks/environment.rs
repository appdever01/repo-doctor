use crate::config::Environment;
use crate::model::{CheckResult, Evidence, Status};
use crate::safety::{FileReader, variable_name};
use std::collections::{BTreeMap, BTreeSet};

pub struct Parsed {
    values: BTreeMap<String, String>,
    duplicates: BTreeSet<String>,
}

fn parse_value(raw: &str) -> Result<String, ()> {
    let raw = raw.trim();
    if let Some(quote) = raw.chars().next().filter(|c| *c == '\'' || *c == '"') {
        let mut value = String::new();
        let mut chars = raw[1..].char_indices();
        while let Some((index, ch)) = chars.next() {
            if ch == quote {
                let tail = raw[index + 2..].trim();
                return if tail.is_empty() || tail.starts_with('#') {
                    Ok(value)
                } else {
                    Err(())
                };
            }
            if ch == '\\' && quote == '"' {
                let (_, escaped) = chars.next().ok_or(())?;
                value.push(match escaped {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    '\\' => '\\',
                    '"' => '"',
                    '$' => '$',
                    _ => return Err(()),
                });
            } else {
                value.push(ch);
            }
        }
        Err(())
    } else {
        let value = raw
            .char_indices()
            .find(|(i, ch)| *ch == '#' && (*i == 0 || raw[..*i].ends_with(char::is_whitespace)))
            .map_or(raw, |(i, _)| &raw[..i])
            .trim_end();
        if value.contains(['\'', '"']) {
            return Err(());
        }
        Ok(value.into())
    }
}

pub fn parse(input: &str) -> Result<Parsed, usize> {
    let mut values = BTreeMap::new();
    let mut duplicates = BTreeSet::new();
    for (i, line) in input.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line
            .strip_prefix("export ")
            .or_else(|| line.strip_prefix("export\t"))
            .unwrap_or(line)
            .trim_start();
        let (key, raw) = line.split_once('=').ok_or(i + 1)?;
        let key = key.trim();
        if !variable_name(key) {
            return Err(i + 1);
        }
        let value = parse_value(raw).map_err(|_| i + 1)?;
        if values.insert(key.into(), value).is_some() {
            duplicates.insert(key.into());
        }
    }
    Ok(Parsed { values, duplicates })
}

fn load(
    reader: &mut FileReader,
    file: &str,
    id: &str,
    explicit: bool,
    checks: &mut Vec<CheckResult>,
) -> Result<Option<Parsed>, ()> {
    match reader.read(file) {
        Ok(Some(input)) => match parse(&input) {
            Ok(parsed) => {
                for name in &parsed.duplicates {
                    checks.push(
                        CheckResult::new(
                            &format!("{id}.duplicate.{name}"),
                            Status::Warn,
                            "env.duplicate",
                            format!("{name} is assigned more than once; the last assignment wins."),
                            file,
                        )
                        .next("Keep one assignment for this variable in this file."),
                    );
                }
                Ok(Some(parsed))
            }
            Err(line) => {
                checks.push(CheckResult::new(id, Status::Error, "env.parse_error", format!("Unsupported environment syntax at line {line}. No values were reported."), file).next("Use one ordinary key=value assignment per line, with optional quotes or export."));
                Err(())
            }
        },
        Ok(None) if !explicit => Ok(None),
        Ok(None) => {
            checks.push(CheckResult::new(
                id,
                Status::Error,
                "env.file_missing",
                "The explicitly selected environment file is missing.",
                file,
            ));
            Err(())
        }
        Err(reason) => {
            checks.push(CheckResult::new(
                id,
                Status::Error,
                "env.read_error",
                reason,
                file,
            ));
            Err(())
        }
    }
}

pub fn check(reader: &mut FileReader, config: &Environment) -> Vec<CheckResult> {
    let mut checks = vec![];
    let template = config.template.as_deref().unwrap_or(".env.example");
    let mut required: BTreeMap<String, Vec<String>> = config
        .required
        .iter()
        .map(|n| (n.clone(), vec![".repo-doctor.toml".into()]))
        .collect();
    if let Ok(Some(parsed)) = load(
        reader,
        template,
        "env.template",
        config.template.is_some(),
        &mut checks,
    ) {
        for key in parsed.values.keys() {
            if !config.optional.contains(key) {
                required
                    .entry(key.clone())
                    .or_default()
                    .push(template.into());
            }
        }
    }
    let mut values = BTreeMap::new();
    let mut reliable = true;
    for (i, file) in config.files.iter().enumerate() {
        match load(reader, file, &format!("env.file.{i}"), false, &mut checks) {
            Ok(Some(parsed)) => values.extend(parsed.values),
            Ok(None) => (),
            Err(()) => reliable = false,
        }
    }
    for (name, sources) in required {
        let inherited = std::env::var_os(&name);
        let present = if let Some(value) = inherited {
            Some(!value.is_empty())
        } else {
            values.get(&name).map(|value| !value.is_empty())
        };
        let id = format!("env.{name}");
        let source = sources.first().map(String::as_str).unwrap_or(template);
        let mut check = if !reliable && std::env::var_os(&name).is_none() {
            CheckResult::new(
                &id,
                Status::Skipped,
                "env.prerequisite_unverified",
                format!(
                    "{name} could not be verified because a value file was unreadable or malformed."
                ),
                source,
            )
        } else {
            match present {
                Some(true) => CheckResult::new(
                    &id,
                    Status::Pass,
                    "env.present",
                    format!("{name} is set."),
                    source,
                ),
                Some(false) => CheckResult::new(
                    &id,
                    Status::Fail,
                    "env.empty",
                    format!("{name} is empty."),
                    source,
                )
                .next(format!(
                    "Set {name} in your selected environment file or shell."
                )),
                None => CheckResult::new(
                    &id,
                    Status::Fail,
                    "env.missing",
                    format!("{name} is missing."),
                    source,
                )
                .next(format!(
                    "Set {name} in your selected environment file or shell."
                )),
            }
        };
        check.evidence = sources
            .into_iter()
            .map(|source| Evidence {
                source,
                detail: "Required variable name".into(),
            })
            .collect();
        checks.push(check);
    }
    checks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_duplicates_and_literal_substitutions() {
        let parsed = parse("# comment\nexport TOKEN='synthetic value'\nTOKEN=last\nX=\"$TOKEN $(touch sentinel)\"\nY=\"escaped\\nvalue\"\n").unwrap();
        assert_eq!(parsed.values["TOKEN"], "last");
        assert!(parsed.duplicates.contains("TOKEN"));
        assert_eq!(parsed.values["X"], "$TOKEN $(touch sentinel)");
        assert_eq!(parsed.values["Y"], "escaped\nvalue");
        assert_eq!(parse("KEY=\"unclosed").err(), Some(1));
        assert_eq!(parse("BAD-NAME=value").err(), Some(1));
    }
}
