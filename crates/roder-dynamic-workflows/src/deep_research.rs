use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const DEEP_RESEARCH_COMMAND_NAME: &str = "deep-research";

pub fn deep_research_workflow_source() -> &'static str {
    DEEP_RESEARCH_WORKFLOW_SOURCE
}

pub fn deep_research_arguments(
    question: impl Into<String>,
    provider: Option<&dyn DeepResearchSearchProvider>,
) -> Value {
    let question = question.into();
    let seed_results = provider
        .map(|provider| provider.search(&question))
        .unwrap_or_default();
    json!({
        "question": question,
        "seedResults": seed_results
    })
}

pub trait DeepResearchSearchProvider {
    fn search(&self, query: &str) -> Vec<DeepResearchSearchResult>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeepResearchSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeepResearchFixtureSearchProvider {
    pub results: Vec<DeepResearchSearchResult>,
}

impl DeepResearchFixtureSearchProvider {
    pub fn from_json_str(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }
}

impl DeepResearchSearchProvider for DeepResearchFixtureSearchProvider {
    fn search(&self, query: &str) -> Vec<DeepResearchSearchResult> {
        let query_terms = normalized_terms(query);
        let mut scored = self
            .results
            .iter()
            .map(|result| (fixture_score(result, &query_terms), result))
            .filter(|(score, _)| *score > 0)
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.title.cmp(&right.1.title)));
        if scored.is_empty() {
            return self.results.iter().take(4).cloned().collect();
        }
        scored
            .into_iter()
            .take(4)
            .map(|(_, result)| result.clone())
            .collect()
    }
}

fn normalized_terms(value: &str) -> Vec<String> {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(str::to_ascii_lowercase)
        .filter(|term| term.len() > 2)
        .collect()
}

fn fixture_score(result: &DeepResearchSearchResult, query_terms: &[String]) -> usize {
    let text = format!("{} {}", result.title, result.snippet).to_ascii_lowercase();
    query_terms
        .iter()
        .filter(|term| text.contains(term.as_str()))
        .count()
}

const DEEP_RESEARCH_WORKFLOW_SOURCE: &str = r###"
workflow.define({
  name: "deep-research",
  description: "Run a multi-agent deep research workflow with web search when available.",
  hostApiVersion: 1,
  argumentsSchema: {
    type: "object",
    additionalProperties: false,
    required: ["question"],
    properties: {
      question: { type: "string", minLength: 1 },
      queries: {
        type: "array",
        items: { type: "string" },
        maxItems: 12
      },
      seedResults: {
        type: "array",
        items: {
          type: "object",
          required: ["title", "url", "snippet"],
          properties: {
            title: { type: "string" },
            url: { type: "string" },
            snippet: { type: "string" }
          }
        }
      },
      webSearchAvailable: {
        type: "boolean"
      }
    }
  },
  phases: ["scope", "parallel-research", "synthesis", "verification"],
  limits: {
    maxConcurrentAgents: 8,
    maxAgentsPerRun: 64,
    defaultAgentTimeoutSeconds: 900,
    defaultRunTimeoutSeconds: 7200,
    maxReportBytes: 65536
  }
}, async (ctx) => {
  const args = ctx.run.arguments || {};
  const question = String(args.question || args.query || args.arguments || "").trim();
  if (!question) {
    throw new Error("deep-research requires a non-empty question");
  }
  const seedResults = Array.isArray(args.seedResults) ? args.seedResults : [];
  const webSearchAvailable = args.webSearchAvailable !== false;
  if (!webSearchAvailable && seedResults.length === 0) {
    throw new Error("deep-research requires web-search capability or fixture seedResults");
  }
  const requestedQueries = Array.isArray(args.queries) && args.queries.length
    ? args.queries.map((query) => String(query)).slice(0, 12)
    : [
      question,
      `${question} current evidence`,
      `${question} opposing evidence`,
      `${question} implementation details`,
      `${question} risks limitations`
    ];

  ctx.phase.start("scope");
  const scope = await ctx.agents.run("research-lead", {
    lane: "planning",
    description: "scope the research question and source strategy",
    prompt: `Scope this deep research question, identify likely source classes, and define acceptance criteria.\n\nQuestion: ${question}`,
    output: `scope:${question}`
  });
  ctx.checkpoint.save("scope", {
    question,
    queryCount: requestedQueries.length,
    seedResultCount: seedResults.length
  });

  ctx.phase.start("parallel-research");
  const researchers = await ctx.agents.map("researcher", requestedQueries, (query, index) => ({
    lane: "research",
    description: `research query ${index + 1}`,
    prompt: `Research this query for the deep research workflow.\n\nQuestion: ${question}\nQuery: ${query}\n\nUse Roder's canonical web-search tools when available. Prefer primary sources, quote sparingly, record URLs, and call out uncertainty. If fixture seed results are present, use them as offline evidence before adding live sources.\n\nFixture seed results:\n${JSON.stringify(seedResults)}`,
    output: `research:${index + 1}:${query}`
  }));
  ctx.checkpoint.save("research", researchers.map((agent) => agent.output));

  ctx.phase.start("synthesis");
  const synthesis = await ctx.agents.run("synthesizer", {
    lane: "synthesis",
    description: "merge findings into a concise research answer",
    prompt: `Synthesize the research outputs into a sourced answer.\n\nQuestion: ${question}\nScope: ${scope.output}\nResearch outputs:\n${researchers.map((agent) => agent.output).join("\n")}`,
    output: `synthesis:${question}`
  });

  ctx.phase.start("verification");
  const verification = await ctx.agents.run("verifier", {
    lane: "verification",
    description: "challenge the synthesis and check source quality",
    prompt: `Verify this synthesis. Check for stale facts, unsupported claims, missing counterarguments, and source-quality issues.\n\nQuestion: ${question}\nSynthesis: ${synthesis.output}`,
    output: `verification:${question}`
  });

  return ctx.report.markdown([
    `# Deep research: ${question}`,
    "",
    `Scope: ${scope.output}`,
    "",
    "## Research lanes",
    researchers.map((agent) => `- ${agent.output}`).join("\n"),
    "",
    `Synthesis: ${synthesis.output}`,
    `Verification: ${verification.output}`
  ]);
});
"###;
