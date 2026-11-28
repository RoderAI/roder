use serde_json::Value;

use super::burst::exercise_burst;
use super::*;

fn percentile(values: &[Duration], percent: usize) -> f64 {
    let index = (values.len() * percent).div_ceil(100) - 1;
    values[index].as_secs_f64() * 1000.0
}

fn summarize(mut samples: Vec<Duration>) -> Value {
    samples.sort_unstable();
    json!({
        "samples": samples.len(),
        "p50_ms": percentile(&samples, 50),
        "p95_ms": percentile(&samples, 95),
        "max_ms": percentile(&samples, 100)
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "offline latency measurement; run explicitly with --ignored --nocapture"]
async fn agent_message_board_measure_scenarios() {
    const REPEATS: usize = 20;
    const POSTS: usize = 16;
    let mut claim = Vec::new();
    let mut direct = Vec::new();
    let mut handoff = Vec::new();
    let mut interrupt = Vec::new();
    let mut post = Vec::new();
    let mut burst = Vec::new();
    let mut idle = Vec::new();
    let mut post_bytes = 0;
    let mut notice_bytes = 0;
    for _ in 0..REPEATS {
        let timing = exercise_handoff(false).await;
        claim.push(timing.claim_race);
        direct.push(timing.direct_delivery);
        handoff.push(timing.handoff_delivery);
        interrupt.push(timing.interrupt_recovery);
        let timing = exercise_burst(POSTS).await;
        post.extend(timing.post_round_trips);
        burst.push(timing.burst_delivery);
        idle.push(timing.idle_recovery);
        post_bytes += timing.post_body_bytes;
        notice_bytes += timing.notice_bytes;
    }
    let results = json!({
        "repeats_per_scenario": REPEATS,
        "posts_per_burst": POSTS,
        "claim_race": summarize(claim),
        "active_direct_mail": summarize(direct),
        "active_handoff_mail": summarize(handoff),
        "recover_interrupted_parent": summarize(interrupt),
        "post_result": summarize(post),
        "first_post_to_final_notice": summarize(burst),
        "idle_notice_after_restart": summarize(idle),
        "board_post_body_bytes": post_bytes,
        "mailbox_notice_body_bytes": notice_bytes
    });
    println!("agent_message_board_metrics {results}");
}
