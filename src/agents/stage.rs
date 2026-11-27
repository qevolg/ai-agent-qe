use anyhow::Result;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    Explorer,
    Hunter,
    Designer,
    Developer,
    Validator,
}

impl Stage {
    pub const ALL: [Self; 5] = [
        Self::Explorer,
        Self::Hunter,
        Self::Designer,
        Self::Developer,
        Self::Validator,
    ];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Explorer => "code-explorer",
            Self::Hunter => "bug-hunter",
            Self::Designer => "test-designer",
            Self::Developer => "test-developer",
            Self::Validator => "test-validator",
        }
    }
    pub fn parse(name: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|s| s.name() == name)
            .ok_or_else(|| anyhow::anyhow!("Unknown stage: {name}"))
    }
    pub const fn resources(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Explorer => (
                include_str!("../../resources/agents/code-explorer.md"),
                include_str!("../../resources/skills/code-explorer/SKILL.md"),
                include_str!("../../resources/schemas/code-explorer.json"),
            ),
            Self::Hunter => (
                include_str!("../../resources/agents/bug-hunter.md"),
                include_str!("../../resources/skills/bug-hunter/SKILL.md"),
                include_str!("../../resources/schemas/bug-hunter.json"),
            ),
            Self::Designer => (
                include_str!("../../resources/agents/test-designer.md"),
                include_str!("../../resources/skills/test-designer/SKILL.md"),
                include_str!("../../resources/schemas/test-designer.json"),
            ),
            Self::Developer => (
                include_str!("../../resources/agents/test-developer.md"),
                include_str!("../../resources/skills/test-developer/SKILL.md"),
                include_str!("../../resources/schemas/test-developer.json"),
            ),
            Self::Validator => (
                include_str!("../../resources/agents/test-validator.md"),
                include_str!("../../resources/skills/test-validator/SKILL.md"),
                include_str!("../../resources/schemas/test-validator.json"),
            ),
        }
    }
}
pub const STAGES: [&str; 5] = [
    Stage::Explorer.name(),
    Stage::Hunter.name(),
    Stage::Designer.name(),
    Stage::Developer.name(),
    Stage::Validator.name(),
];
