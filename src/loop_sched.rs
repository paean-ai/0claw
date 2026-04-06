use crate::mcp::ToolSpec;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
#[allow(dead_code)]
pub struct LoopPromptEvent {
    pub job_id: String,
    pub prompt: String,
    pub schedule: String,
}

struct LoopJob {
    id: String,
    schedule: String,
    prompt: String,
    _interval_secs: u64,
    run_count: u64,
    skip_count: u64,
    paused: bool,
    cancel: CancellationToken,
}

type JobRegistry = Arc<Mutex<HashMap<String, LoopJob>>>;

static REGISTRY: OnceLock<JobRegistry> = OnceLock::new();
static CHANNEL: OnceLock<mpsc::UnboundedSender<LoopPromptEvent>> = OnceLock::new();
static RECEIVER: OnceLock<Mutex<Option<mpsc::UnboundedReceiver<LoopPromptEvent>>>> = OnceLock::new();

fn registry() -> &'static JobRegistry {
    REGISTRY.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

fn init_channel() -> &'static mpsc::UnboundedSender<LoopPromptEvent> {
    CHANNEL.get_or_init(|| {
        let (tx, rx) = mpsc::unbounded_channel();
        RECEIVER.get_or_init(|| Mutex::new(Some(rx)));
        tx
    })
}

pub fn take_receiver() -> Option<mpsc::UnboundedReceiver<LoopPromptEvent>> {
    init_channel(); // ensure channel exists
    RECEIVER.get().and_then(|m| m.lock().ok()?.take())
}

/// Return tool definitions that the agent can discover.
pub fn tool_definitions() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "loop_create".into(),
            description: "Create a recurring scheduled task. The prompt will be injected into the conversation at the specified interval. Schedule format: '30s', '5m', '1h', '2h30m'. The task runs in the current session and stops when 0claw exits.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "schedule": {
                        "type": "string",
                        "description": "Interval like '30s', '5m', '1h', '2h30m'"
                    },
                    "prompt": {
                        "type": "string",
                        "description": "The prompt to execute on each iteration"
                    }
                },
                "required": ["schedule", "prompt"]
            }),
        },
        ToolSpec {
            name: "loop_list".into(),
            description: "List all active loop jobs with their ID, schedule, prompt, run count, and status.".into(),
            parameters: json!({"type": "object", "properties": {}}),
        },
        ToolSpec {
            name: "loop_remove".into(),
            description: "Remove and stop a loop job by its ID.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "job_id": { "type": "string", "description": "The job ID to remove" }
                },
                "required": ["job_id"]
            }),
        },
        ToolSpec {
            name: "loop_pause".into(),
            description: "Pause a loop job. It stays registered but won't fire until resumed.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "job_id": { "type": "string", "description": "The job ID to pause" }
                },
                "required": ["job_id"]
            }),
        },
        ToolSpec {
            name: "loop_resume".into(),
            description: "Resume a paused loop job.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "job_id": { "type": "string", "description": "The job ID to resume" }
                },
                "required": ["job_id"]
            }),
        },
    ]
}

/// Handle a loop_* tool call. Returns the result string.
pub async fn handle_tool_call(name: &str, args_str: &str) -> String {
    let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
    match name {
        "loop_create" => create_job(&args),
        "loop_list" => list_jobs(),
        "loop_remove" => remove_job(&args),
        "loop_pause" => pause_job(&args),
        "loop_resume" => resume_job(&args),
        _ => format!("Unknown loop tool: {name}"),
    }
}

fn create_job(args: &Value) -> String {
    let schedule = args["schedule"].as_str().unwrap_or("");
    let prompt = args["prompt"].as_str().unwrap_or("");

    if schedule.is_empty() || prompt.is_empty() {
        return "Error: schedule and prompt are required".into();
    }

    let secs = match parse_interval(schedule) {
        Some(s) if s >= 5 => s,
        Some(_) => return "Error: minimum interval is 5s".into(),
        None => return format!("Error: cannot parse schedule '{schedule}'. Use formats like '30s', '5m', '1h', '2h30m'"),
    };

    let id = format!("{:08x}", rand_u32());
    let cancel = CancellationToken::new();

    let job = LoopJob {
        id: id.clone(),
        schedule: schedule.into(),
        prompt: prompt.into(),
        _interval_secs: secs,
        run_count: 0,
        skip_count: 0,
        paused: false,
        cancel: cancel.clone(),
    };

    registry().lock().unwrap().insert(id.clone(), job);

    // Spawn executor
    let job_id = id.clone();
    let job_prompt = prompt.to_string();
    let job_schedule = schedule.to_string();
    let tx = init_channel().clone();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(std::time::Duration::from_secs(secs)) => {}
            }

            if cancel.is_cancelled() {
                break;
            }

            let paused = registry()
                .lock()
                .unwrap()
                .get(&job_id)
                .map(|j| j.paused)
                .unwrap_or(true);

            if paused {
                continue;
            }

            if crate::cli::is_agent_busy() {
                if let Some(job) = registry().lock().unwrap().get_mut(&job_id) {
                    job.skip_count += 1;
                }
                continue;
            }

            let _ = tx.send(LoopPromptEvent {
                job_id: job_id.clone(),
                prompt: job_prompt.clone(),
                schedule: job_schedule.clone(),
            });

            if let Some(job) = registry().lock().unwrap().get_mut(&job_id) {
                job.run_count += 1;
            }
        }
    });

    format!("Loop created: id={id}, schedule=every {schedule}, prompt=\"{prompt}\"")
}

fn list_jobs() -> String {
    let reg = registry().lock().unwrap();
    if reg.is_empty() {
        return "No active loop jobs.".into();
    }
    let mut lines = Vec::new();
    for job in reg.values() {
        let status = if job.paused { "paused" } else { "active" };
        lines.push(format!(
            "- {} | every {} | runs:{} skips:{} | {} | \"{}\"",
            job.id, job.schedule, job.run_count, job.skip_count, status,
            job.prompt.chars().take(60).collect::<String>()
        ));
    }
    lines.join("\n")
}

fn remove_job(args: &Value) -> String {
    let id = args["job_id"].as_str().unwrap_or("");
    let mut reg = registry().lock().unwrap();
    if let Some(job) = reg.remove(id) {
        job.cancel.cancel();
        format!("Removed loop job {id}")
    } else {
        format!("Job not found: {id}")
    }
}

fn pause_job(args: &Value) -> String {
    let id = args["job_id"].as_str().unwrap_or("");
    let mut reg = registry().lock().unwrap();
    if let Some(job) = reg.get_mut(id) {
        job.paused = true;
        format!("Paused loop job {id}")
    } else {
        format!("Job not found: {id}")
    }
}

fn resume_job(args: &Value) -> String {
    let id = args["job_id"].as_str().unwrap_or("");
    let mut reg = registry().lock().unwrap();
    if let Some(job) = reg.get_mut(id) {
        job.paused = false;
        format!("Resumed loop job {id}")
    } else {
        format!("Job not found: {id}")
    }
}

/// Parse duration strings like "30s", "5m", "1h", "2h30m", "1h15m30s".
fn parse_interval(s: &str) -> Option<u64> {
    let s = s.trim().to_lowercase();
    let mut total: u64 = 0;
    let mut num_buf = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            num_buf.push(c);
        } else {
            let n: u64 = num_buf.parse().ok()?;
            num_buf.clear();
            match c {
                'h' => total += n * 3600,
                'm' => total += n * 60,
                's' => total += n,
                _ => return None,
            }
        }
    }
    if !num_buf.is_empty() {
        // Bare number defaults to seconds
        let n: u64 = num_buf.parse().ok()?;
        total += n;
    }
    if total == 0 { None } else { Some(total) }
}

fn rand_u32() -> u32 {
    use std::time::SystemTime;
    let t = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    // Simple hash from time nanos
    let n = t.as_nanos() as u64;
    ((n ^ (n >> 17) ^ (n >> 33)).wrapping_mul(0x9E3779B97F4A7C15)) as u32
}
