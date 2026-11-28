use anyhow::Result;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    Explorer,
    Hunter,
    Designer,
    Developer,
    Validator,
    ProductExplorer,
    ScenarioDesigner,
    ProductExecutor,
    IssueReproducer,
    ResultReviewer,
}

impl Stage {
    pub const ALL: [Self; 5] = [
        Self::Explorer,
        Self::Hunter,
        Self::Designer,
        Self::Developer,
        Self::Validator,
    ];

    pub const PRODUCT: [Self; 5] = [
        Self::ProductExplorer,
        Self::ScenarioDesigner,
        Self::ProductExecutor,
        Self::IssueReproducer,
        Self::ResultReviewer,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Explorer => "code-explorer",
            Self::Hunter => "bug-hunter",
            Self::Designer => "test-designer",
            Self::Developer => "test-developer",
            Self::Validator => "test-validator",
            Self::ProductExplorer => "product-explorer",
            Self::ScenarioDesigner => "scenario-designer",
            Self::ProductExecutor => "product-executor",
            Self::IssueReproducer => "issue-reproducer",
            Self::ResultReviewer => "result-reviewer",
        }
    }

    pub fn parse(name: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .chain(Self::PRODUCT)
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
            Self::ProductExplorer => (
                include_str!("../../resources/agents/product-explorer.md"),
                include_str!("../../resources/skills/product-explorer/SKILL.md"),
                include_str!("../../resources/schemas/product-explorer.json"),
            ),
            Self::ScenarioDesigner => (
                include_str!("../../resources/agents/scenario-designer.md"),
                include_str!("../../resources/skills/scenario-designer/SKILL.md"),
                include_str!("../../resources/schemas/scenario-designer.json"),
            ),
            Self::ProductExecutor => (
                include_str!("../../resources/agents/product-executor.md"),
                include_str!("../../resources/skills/product-executor/SKILL.md"),
                include_str!("../../resources/schemas/product-executor.json"),
            ),
            Self::IssueReproducer => (
                include_str!("../../resources/agents/issue-reproducer.md"),
                include_str!("../../resources/skills/issue-reproducer/SKILL.md"),
                include_str!("../../resources/schemas/issue-reproducer.json"),
            ),
            Self::ResultReviewer => (
                include_str!("../../resources/agents/result-reviewer.md"),
                include_str!("../../resources/skills/result-reviewer/SKILL.md"),
                include_str!("../../resources/schemas/result-reviewer.json"),
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

pub const PRODUCT_STAGES: [&str; 5] = [
    Stage::ProductExplorer.name(),
    Stage::ScenarioDesigner.name(),
    Stage::ProductExecutor.name(),
    Stage::IssueReproducer.name(),
    Stage::ResultReviewer.name(),
];
