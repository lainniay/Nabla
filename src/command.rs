use std::collections::HashSet;

use serde::{Deserialize, Serialize};

pub const COMMAND_MENU_VISIBLE_ROWS: usize = 7;

const LOCAL_COMMANDS: [LocalCommandDefinition; 17] = [
    LocalCommandDefinition::new(
        "login",
        "Show how to authenticate Pi",
        LocalCommandKind::Login,
    ),
    LocalCommandDefinition::new(
        "compact",
        "Compact the current context",
        LocalCommandKind::Compact,
    ),
    LocalCommandDefinition::new(
        "context",
        "Inspect the current context budget",
        LocalCommandKind::Context,
    ),
    LocalCommandDefinition::new(
        "resources",
        "Inspect loaded context, skills, prompts, and extensions",
        LocalCommandKind::Resources,
    ),
    LocalCommandDefinition::new("reload", "Reload Pi resources", LocalCommandKind::Reload),
    LocalCommandDefinition::new(
        "trust",
        "Show or change project resource trust",
        LocalCommandKind::Trust,
    ),
    LocalCommandDefinition::new(
        "goal",
        "Start or control a structured Goal",
        LocalCommandKind::Goal,
    ),
    LocalCommandDefinition::new(
        "goals",
        "Inspect the Goal sidecar for this session",
        LocalCommandKind::Goals,
    ),
    LocalCommandDefinition::new("model", "List or select a model", LocalCommandKind::Model),
    LocalCommandDefinition::new(
        "thinking",
        "Show or change the thinking level",
        LocalCommandKind::Thinking,
    ),
    LocalCommandDefinition::new(
        "agents",
        "Inspect configured and running subagents",
        LocalCommandKind::Agents,
    ),
    LocalCommandDefinition::new(
        "agent",
        "Start a configured subagent",
        LocalCommandKind::Agent,
    ),
    LocalCommandDefinition::new(
        "plan",
        "Enter Plan mode or control a submitted Plan",
        LocalCommandKind::Plan,
    ),
    LocalCommandDefinition::new("help", "List available commands", LocalCommandKind::Help),
    LocalCommandDefinition::new("new", "Start a new session", LocalCommandKind::New),
    LocalCommandDefinition::new(
        "resume",
        "Resume a previous session",
        LocalCommandKind::Resume,
    ),
    LocalCommandDefinition::new(
        "tree",
        "Navigate the current session tree",
        LocalCommandKind::Tree,
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalCommandKind {
    Login,
    New,
    Resume,
    Tree,
    Plan,
    Compact,
    Context,
    Resources,
    Reload,
    Trust,
    Goal,
    Goals,
    Model,
    Thinking,
    Agents,
    Agent,
    Help,
}

#[derive(Debug, Clone, Copy)]
struct LocalCommandDefinition {
    name: &'static str,
    description: &'static str,
    kind: LocalCommandKind,
}

impl LocalCommandDefinition {
    const fn new(name: &'static str, description: &'static str, kind: LocalCommandKind) -> Self {
        Self {
            name,
            description,
            kind,
        }
    }
}

impl LocalCommandKind {
    fn parse(self, argument: Option<String>) -> LocalCommand {
        match self {
            Self::Login => LocalCommand::Login,
            Self::New => LocalCommand::New(argument),
            Self::Resume => LocalCommand::Resume(argument),
            Self::Tree => LocalCommand::Tree(argument),
            Self::Plan => LocalCommand::Plan(argument),
            Self::Compact => LocalCommand::Compact(argument),
            Self::Context => LocalCommand::Context,
            Self::Resources => LocalCommand::Resources,
            Self::Reload => LocalCommand::Reload,
            Self::Trust => LocalCommand::Trust(argument),
            Self::Goal => LocalCommand::Goal(argument),
            Self::Goals => LocalCommand::Goals,
            Self::Model => LocalCommand::Model(argument),
            Self::Thinking => LocalCommand::Thinking(argument),
            Self::Agents => LocalCommand::Agents(argument),
            Self::Agent => LocalCommand::Agent(argument),
            Self::Help => LocalCommand::Help,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSource {
    Local,
    Extension,
    Prompt,
    Skill,
    Unknown,
}

impl CommandSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Extension => "extension",
            Self::Prompt => "prompt",
            Self::Skill => "skill",
            Self::Unknown => "pi",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct DiscoveredCommand {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub name: String,
    pub description: String,
    pub source: CommandSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalCommand {
    Login,
    New(Option<String>),
    Resume(Option<String>),
    Tree(Option<String>),
    Plan(Option<String>),
    Compact(Option<String>),
    Context,
    Resources,
    Reload,
    Trust(Option<String>),
    Goal(Option<String>),
    Goals,
    Model(Option<String>),
    Thinking(Option<String>),
    Agents(Option<String>),
    Agent(Option<String>),
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandRoute {
    Local(LocalCommand),
    Prompt,
    Unknown {
        name: String,
        suggestions: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub struct CommandCatalog {
    commands: Vec<CommandSpec>,
}

impl Default for CommandCatalog {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl CommandCatalog {
    pub fn new(discovered: Vec<DiscoveredCommand>) -> Self {
        let mut commands = LOCAL_COMMANDS
            .into_iter()
            .map(|definition| CommandSpec {
                name: definition.name.to_owned(),
                description: definition.description.to_owned(),
                source: CommandSource::Local,
            })
            .collect::<Vec<_>>();
        let mut names = commands
            .iter()
            .map(|command| command.name.clone())
            .collect::<HashSet<_>>();

        let mut discovered = discovered
            .into_iter()
            .filter_map(|command| {
                let name = command.name.trim().trim_start_matches('/');
                if name.is_empty() || name.chars().any(char::is_whitespace) {
                    return None;
                }
                Some(CommandSpec {
                    name: name.to_owned(),
                    description: command.description,
                    source: parse_source(&command.source),
                })
            })
            .filter(|command| names.insert(command.name.clone()))
            .collect::<Vec<_>>();
        discovered.sort_by(|left, right| left.name.cmp(&right.name));
        commands.extend(discovered);

        Self { commands }
    }

    pub fn commands(&self) -> &[CommandSpec] {
        &self.commands
    }

    pub fn route(&self, source: &str) -> CommandRoute {
        let trimmed = source.trim();
        let Some(command_line) = trimmed.strip_prefix('/') else {
            return CommandRoute::Prompt;
        };
        let mut parts = command_line.splitn(2, char::is_whitespace);
        let name = parts.next().unwrap_or_default();
        let argument = parts
            .next()
            .map(str::trim)
            .filter(|argument| !argument.is_empty())
            .map(ToOwned::to_owned);

        match LOCAL_COMMANDS.iter().find(|command| command.name == name) {
            Some(command) => CommandRoute::Local(command.kind.parse(argument)),
            _ if self
                .commands
                .iter()
                .any(|command| command.source != CommandSource::Local && command.name == name) =>
            {
                CommandRoute::Prompt
            }
            _ => CommandRoute::Unknown {
                name: name.to_owned(),
                suggestions: self
                    .commands
                    .iter()
                    .filter(|command| command.name.starts_with(name))
                    .take(3)
                    .map(|command| command.name.clone())
                    .collect(),
            },
        }
    }

    pub fn suggestion(&self, input: &str, cursor: usize) -> Option<&CommandSpec> {
        self.candidates(input, cursor, 1).into_iter().next()
    }

    pub fn candidates(&self, input: &str, cursor: usize, limit: usize) -> Vec<&CommandSpec> {
        if cursor != input.chars().count() {
            return Vec::new();
        }
        let Some(prefix) = input.strip_prefix('/') else {
            return Vec::new();
        };
        if prefix.chars().any(char::is_whitespace) {
            return Vec::new();
        }
        let prefix = prefix.to_ascii_lowercase();
        let mut matches = self
            .commands
            .iter()
            .filter(|command| command.name.to_ascii_lowercase().starts_with(&prefix))
            .collect::<Vec<_>>();
        matches.sort_by_key(|command| command.name.to_ascii_lowercase() != prefix);
        matches.truncate(limit);
        matches
    }

    pub fn completion(&self, input: &str, cursor: usize) -> Option<String> {
        let command = self.suggestion(input, cursor)?;
        Some(format!("/{} ", command.name))
    }

    pub fn help_text(&self) -> String {
        self.commands
            .iter()
            .map(|command| {
                format!(
                    "/{:<18} {}  [{}]",
                    command.name,
                    command.description,
                    command.source.label()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn parse_source(source: &str) -> CommandSource {
    match source {
        "extension" => CommandSource::Extension,
        "prompt" => CommandSource::Prompt,
        "skill" => CommandSource::Skill,
        _ => CommandSource::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> CommandCatalog {
        CommandCatalog::new(vec![
            DiscoveredCommand {
                name: "fix-tests".to_owned(),
                description: "Fix failing tests".to_owned(),
                source: "prompt".to_owned(),
            },
            DiscoveredCommand {
                name: "review".to_owned(),
                description: "Review changes".to_owned(),
                source: "extension".to_owned(),
            },
        ])
    }

    #[test]
    fn routes_local_dynamic_plain_and_unknown_inputs() {
        let catalog = catalog();

        assert_eq!(
            catalog.route("/compact keep decisions"),
            CommandRoute::Local(LocalCommand::Compact(Some("keep decisions".to_owned())))
        );
        assert_eq!(
            catalog.route("/plan"),
            CommandRoute::Local(LocalCommand::Plan(None))
        );
        assert_eq!(
            catalog.route("/context"),
            CommandRoute::Local(LocalCommand::Context)
        );
        assert_eq!(catalog.route("/fix-tests src"), CommandRoute::Prompt);
        assert_eq!(catalog.route("fix tests"), CommandRoute::Prompt);
        assert_eq!(
            catalog.route("/rev"),
            CommandRoute::Unknown {
                name: "rev".to_owned(),
                suggestions: vec!["review".to_owned()],
            }
        );
    }

    #[test]
    fn local_commands_win_over_discovered_name_collisions() {
        let catalog = CommandCatalog::new(vec![DiscoveredCommand {
            name: "compact".to_owned(),
            description: "Remote compact".to_owned(),
            source: "extension".to_owned(),
        }]);

        assert_eq!(
            catalog
                .commands()
                .iter()
                .filter(|command| command.name == "compact")
                .count(),
            1
        );
        assert!(matches!(
            catalog.route("/compact"),
            CommandRoute::Local(LocalCommand::Compact(None))
        ));
    }

    #[test]
    fn completion_only_applies_to_an_unbroken_command_at_the_cursor() {
        let catalog = catalog();

        assert_eq!(catalog.completion("/fi", 3).as_deref(), Some("/fix-tests "));
        assert!(catalog.completion("/fi argument", 12).is_none());
        assert!(catalog.completion("/fi", 1).is_none());
    }

    #[test]
    fn exact_agent_command_precedes_the_agents_prefix_match() {
        let catalog = catalog();
        assert_eq!(
            catalog
                .suggestion("/agent", 6)
                .map(|command| command.name.as_str()),
            Some("agent")
        );
    }

    #[test]
    fn candidates_are_prefix_filtered_and_limited() {
        let catalog = catalog();

        let all = catalog.candidates("/", 1, 2);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "login");

        let filtered = catalog.candidates("/F", 2, COMMAND_MENU_VISIBLE_ROWS);
        assert_eq!(
            filtered
                .iter()
                .map(|command| command.name.as_str())
                .collect::<Vec<_>>(),
            vec!["fix-tests"]
        );
    }
}
