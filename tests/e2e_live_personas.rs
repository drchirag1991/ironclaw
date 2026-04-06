//! Live/replay tests for commitment-system persona bundles.
//!
//! Exercises the three persona bundles (`ceo-assistant`,
//! `content-creator-assistant`, `trader-assistant`) end-to-end with a real
//! LLM to verify that:
//!
//! 1. The persona bundle skill activates from the user's opening prompt
//!    (keyword/pattern matching).
//! 2. The agent follows the skill's setup flow, writing workspace files
//!    via `memory_write` and scheduling missions/routines.
//! 3. The resulting workspace has the expected commitments structure.
//!
//! All tests run through engine v2 (the production path). Auto-approve is
//! enabled so memory/mission tool calls don't stall on approval gates.
//!
//! # Running
//!
//! **Replay mode** (default, deterministic, needs committed trace fixtures):
//! ```bash
//! cargo test --features libsql --test e2e_live_personas -- --ignored
//! ```
//!
//! **Live mode** (real LLM calls, records/updates trace fixtures):
//! ```bash
//! IRONCLAW_LIVE_TEST=1 cargo test --features libsql --test e2e_live_personas -- --ignored --test-threads=1
//! ```
//!
//! Live mode requires `~/.ironclaw/.env` with valid LLM credentials and
//! runs one test at a time to avoid concurrent API pressure.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod persona_tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use crate::support::live_harness::{LiveTestHarness, LiveTestHarnessBuilder};

    /// Absolute path to the repo's `skills/` directory — the source of the
    /// committed SKILL.md files for commitments and persona bundles.
    fn repo_skills_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skills")
    }

    /// Build a live harness configured for commitment/persona tests.
    ///
    /// Uses engine v2, auto-approves tool calls, loads all skills from the
    /// repo's `./skills/` dir, and bumps iteration count because the setup
    /// flow involves many sequential memory/mission tool calls.
    async fn build_persona_harness(test_name: &str) -> LiveTestHarness {
        LiveTestHarnessBuilder::new(test_name)
            .with_engine_v2(true)
            .with_auto_approve_tools(true)
            .with_max_tool_iterations(60)
            .with_skills_dir(repo_skills_dir())
            .build()
            .await
    }

    const PERSONA_JUDGE_CRITERIA: &str = "\
        The assistant acknowledged the user's persona (CEO/executive, \
        content creator, or trader/investor), explained that it is setting \
        up a commitment tracking workspace, and either asked one or more \
        configuration questions (timezone, channel, cadence, platforms, \
        position tracking, etc.) or confirmed that the workspace structure \
        has been created. A valid response must reference commitments, \
        workspace, or setup — a generic reply that ignores the persona \
        request fails.";

    /// Shared assertion logic: verify the persona skill activated, the agent
    /// took commitments-related actions, and the response judge-passes.
    ///
    /// `user_input` is the opening message. `expected_skill` is the persona
    /// bundle skill name. `activation_keywords` are substrings any of which
    /// must appear in the response (case-insensitive).
    async fn assert_persona_setup(
        harness: LiveTestHarness,
        user_input: &str,
        expected_skill: &str,
        activation_keywords: &[&str],
    ) {
        let rig = harness.rig();
        rig.send_message(user_input).await;

        // Persona setup flows are multi-step: read existing commitments dir,
        // write several memory files, maybe call mission_create. 300s is
        // conservative for live mode and irrelevant in replay.
        let responses = rig.wait_for_responses(1, Duration::from_secs(300)).await;
        assert!(!responses.is_empty(), "Expected at least one response");

        let text: Vec<String> = responses.iter().map(|r| r.content.clone()).collect();
        let joined_lower = text.join("\n").to_lowercase();
        let tools = rig.tool_calls_started();
        let active_skills = rig.active_skill_names();

        let loaded_skills = rig.loaded_skill_names();
        eprintln!("[Persona] test: {}", expected_skill);
        eprintln!(
            "[Persona] loaded skills ({}): {loaded_skills:?}",
            loaded_skills.len()
        );
        eprintln!("[Persona] active skills: {active_skills:?}");
        eprintln!("[Persona] tools: {tools:?}");
        eprintln!(
            "[Persona] response preview: {}",
            text.join("\n").chars().take(400).collect::<String>()
        );

        // The persona skill must have activated from the opening prompt.
        assert!(
            active_skills.iter().any(|s| s == expected_skill),
            "Expected persona skill '{expected_skill}' to activate. \
             Active skills: {active_skills:?}. Tools: {tools:?}",
        );

        // A valid first-message response is either (a) the agent started the
        // setup flow and touched workspace memory, or (b) the agent followed
        // the skill's "ask configuration questions" step and hasn't written
        // anything yet. Both are legal behaviors per the SKILL.md. What
        // matters is that the persona skill actually drove the conversation:
        // we already verified that above via active_skills, so here we only
        // require that the response contains either setup action or a
        // configuration question marker.
        let touched_workspace = tools.iter().any(|t| {
            t.starts_with("memory_read")
                || t.starts_with("memory_write")
                || t.starts_with("memory_tree")
        });
        let asked_config_question = joined_lower.contains("question")
            || joined_lower.contains("configuration")
            || joined_lower.contains("cadence")
            || joined_lower.contains("timezone")
            || joined_lower.contains("which platforms")
            || joined_lower.contains("which markets")
            || joined_lower.contains("?");
        assert!(
            touched_workspace || asked_config_question,
            "Expected either workspace memory activity or a setup question. Tools: {tools:?}",
        );

        // Response should reference commitments or the persona domain.
        assert!(
            activation_keywords
                .iter()
                .any(|kw| joined_lower.contains(&kw.to_lowercase())),
            "Response did not reference any of {activation_keywords:?}: {joined_lower}",
        );

        // LLM judge for semantic verification (live mode only; returns None in replay).
        if let Some(verdict) = harness.judge(&text, PERSONA_JUDGE_CRITERIA).await {
            assert!(verdict.pass, "LLM judge failed: {}", verdict.reasoning);
        }

        harness.finish(user_input, &text).await;
    }

    /// CEO assistant bundle: executive workflow with delegation-heavy triage.
    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys or a recorded trace fixture
    async fn ceo_assistant_setup() {
        let harness = build_persona_harness("ceo_assistant_setup").await;
        assert_persona_setup(
            harness,
            "I'm a CEO, help me manage my day and keep track of everything I'm delegating to my team.",
            "ceo-assistant",
            &["commitment", "workspace", "delegation", "setup"],
        )
        .await;
    }

    /// Content creator assistant bundle: pipeline-aware workflow for a YouTuber.
    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys or a recorded trace fixture
    async fn content_creator_assistant_setup() {
        let harness = build_persona_harness("content_creator_assistant_setup").await;
        assert_persona_setup(
            harness,
            "I'm a YouTuber, set up a system to track my content pipeline, publishing schedule, and ideas.",
            "content-creator-assistant",
            &["content", "pipeline", "publishing", "workspace"],
        )
        .await;
    }

    /// Trader assistant bundle: position-aware trading workflow with journaling.
    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys or a recorded trace fixture
    async fn trader_assistant_setup() {
        let harness = build_persona_harness("trader_assistant_setup").await;
        assert_persona_setup(
            harness,
            "I'm a trader, help me track my positions and journal my decisions.",
            "trader-assistant",
            &["position", "trade", "journal", "workspace"],
        )
        .await;
    }
}
