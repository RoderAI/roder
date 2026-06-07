//! Prompt fragments for read-only agentic gbrain retrieval.

pub const AGENTIC_RETRIEVAL_GUIDANCE: &str = r#"Use bt-gbrain tools as read-only navigation over a stable memory snapshot.

Retrieval order:
1. Prefer dreamed ontology, evidence cards, communities, and graph paths when available.
2. Use raw fact search as fallback or audit support, not as the first source of truth.
3. For every answerable claim, bind actor, action, date or as-of state, and source evidence.
4. For contradiction questions, retrieve both sides and report unresolved conflicts explicitly.
5. For as-of/current questions, compare the requested as-of belief with current state and name deltas.
6. For decisions, look for actor, alternatives, rationale, date, and later invalidation/supersession.

Keep searching while evidence is missing. Abstain when support is not in memory. Call respond_to_query only when the answer is evidence-backed or the abstention is justified."#;

pub fn agentic_retrieval_system_prompt() -> &'static str {
    AGENTIC_RETRIEVAL_GUIDANCE
}
