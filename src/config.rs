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
    pub startup: Option<Startup>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema: 1,
            runtimes: Runtimes::default(),
            env: Environment::default(),
            ports: vec![],
            services: vec![],
            startup: None,
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

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StartupMode {
    Tcp,
    Exit,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Startup {
    pub command: Vec<String>,
    pub mode: StartupMode,
    pub address: Option<IpAddr>,
    pub port: Option<u16>,
    #[serde(default = "startup_timeout")]
    pub timeout_seconds: u64,
}

fn startup_timeout() -> u64 {
    10
}

impl Startup {
    fn validate(&self) -> Result<(), &'static str> {
        if self.command.len() < 2
            || self.command.len() > 32
            || self
                .command
                .iter()
                .any(|arg| arg.is_empty() || arg.len() > 1024 || arg.chars().any(char::is_control))
            || self.command.iter().map(String::len).sum::<usize>() > 4096
            || !(1..=10).contains(&self.timeout_seconds)
        {
            return Err(
                "Startup requires 2 to 32 bounded arguments and a timeout from 1 to 10 seconds.",
            );
        }
        match self.command[0].as_str() {
            "node" | "python" | "python3" => {
                if self.command[1].starts_with('-')
                    || !crate::safety::relative_path(&self.command[1])
                {
                    return Err(
                        "Runtime startup requires a relative project entry-point file, followed by its arguments.",
                    );
                }
            }
            "npm" | "pnpm" => {
                if self.command.len() < 3 || self.command[1] != "run" {
                    return Err(
                        "Package-manager startup supports only an explicit run command and script name.",
                    );
                }
                let name = &self.command[2];
                if !name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_:".contains(&c))
                    || name.starts_with('-')
                    || [
                        "install",
                        "preinstall",
                        "postinstall",
                        "prepare",
                        "approve-builds",
                        "rebuild",
                    ]
                    .contains(&name.as_str())
                    || self.command[3..].iter().any(|arg| {
                        arg.starts_with("--ignore-scripts")
                            || arg.starts_with("--enable-pre-post-scripts")
                            || arg.starts_with("--config.")
                    })
                {
                    return Err(
                        "Startup must name an application script without installation, approval or script-policy overrides.",
                    );
                }
            }
            _ => {
                return Err(
                    "Startup supports host node, python, python3, npm run and pnpm run launchers; shell commands are unsupported.",
                );
            }
        }
        match self.mode {
            StartupMode::Tcp
                if self.address.is_some_and(|ip| ip.is_loopback())
                    && self.port.is_some_and(|port| port != 0)
                    && self.timeout_seconds >= 2 =>
            {
                Ok(())
            }
            StartupMode::Exit if self.address.is_none() && self.port.is_none() => Ok(()),
            _ => Err(
                "TCP startup requires a loopback IP, nonzero port and at least 2 seconds; exit startup must omit the address and port.",
            ),
        }
    }
}

impl Config {
    pub fn parse(input: &str) -> Result<Self, &'static str> {
        let config: Self = toml::from_str(input)
            .map_err(|_| "Invalid configuration syntax, fields or value types.")?;
        if config.schema != 1 {
            return Err("Only configuration schema 1 is supported.");
        }
        if let Some(startup) = &config.startup {
            startup.validate()?;
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
