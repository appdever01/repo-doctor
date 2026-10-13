use serde::Deserialize;
use std::collections::BTreeSet;
use std::net::IpAddr;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema: u8,
    #[serde(default)]
    pub runtimes: Runtimes,
    #[serde(default)]
    pub env: Environment,
    #[serde(default)]
    pub ports: Vec<Endpoint>,
    #[serde(default)]
    pub services: Vec<Endpoint>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema: 1,
            runtimes: Runtimes::default(),
            env: Environment::default(),
            ports: vec![],
            services: vec![],
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Runtimes {
    pub node: Option<String>,
    pub python: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    pub template: Option<String>,
    #[serde(default = "default_files")]
    pub files: Vec<String>,
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub optional: Vec<String>,
}

fn default_files() -> Vec<String> {
    vec![".env".into(), ".env.local".into()]
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            template: None,
            files: default_files(),
            required: vec![],
            optional: vec![],
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub name: String,
    pub address: IpAddr,
    pub port: u16,
}

impl Config {
    pub fn parse(input: &str) -> Result<Self, &'static str> {
        let config: Self = toml::from_str(input)
            .map_err(|_| "Invalid configuration syntax, fields or value types.")?;
        if config.schema != 1 {
            return Err("Only configuration schema 1 is supported.");
        }
        if config.ports.len() + config.services.len() > 32 {
            return Err("At most 32 port and service checks are allowed.");
        }
        let mut names = BTreeSet::new();
        for endpoint in config.ports.iter().chain(&config.services) {
            if !crate::safety::identifier(&endpoint.name) || !names.insert(&endpoint.name) {
                return Err("Check names must be unique safe lowercase identifiers.");
            }
            if !endpoint.address.is_loopback() || endpoint.port == 0 {
                return Err("Endpoints require a loopback IP literal and a port from 1 to 65535.");
            }
        }
        if config.env.files.len() > 32 {
            return Err("At most 32 environment value files are allowed.");
        }
        if config
            .env
            .template
            .iter()
            .chain(&config.env.files)
            .any(|p| !crate::safety::relative_path(p))
        {
            return Err("Environment file paths must remain inside the selected project.");
        }
        if config
            .env
            .required
            .iter()
            .chain(&config.env.optional)
            .any(|n| !crate::safety::variable_name(n))
        {
            return Err("Environment variable names must be safe identifiers.");
        }
        if config
            .env
            .required
            .iter()
            .any(|n| config.env.optional.contains(n))
        {
            return Err("A variable cannot be both explicitly required and optional.");
        }
        for (kind, constraint) in [
            ("node", &config.runtimes.node),
            ("python", &config.runtimes.python),
        ] {
            if let Some(value) = constraint {
                crate::checks::runtime::Requirement::parse(kind, value, ".repo-doctor.toml", false)
                    .map_err(
                        |_| "Explicit runtime requirements must use supported numeric ranges.",
                    )?;
            }
        }
        Ok(config)
    }
}
