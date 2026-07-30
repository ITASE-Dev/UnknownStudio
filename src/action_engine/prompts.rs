//! Model-facing instruction constants. Identity constraints are appended in the
//! engine, never in the UI, so a user prompt can't override them.

/// Appended to every free-text image generation prompt.
pub const FACIAL_IDENTITY_CONSTRAINT: &str = "CRITICAL INSTRUCTION: If a reference image or person is involved, treat the subject's facial identity as a hard constraint. Analyze and strictly adhere to specific facial landmarks (eye shape, nose structure, jawline, and unique asymmetry). Do NOT 'beautify' or blend these features with generic models. The original face, hair, and beard must remain entirely unaltered.";

/// Appended to any action that may regenerate video frames.
pub const INPAINT_IDENTITY_CONSTRAINT: &str = "CRITICAL INSTRUCTION: Treat the subject's facial identity as a hard constraint. Analyze and strictly adhere to specific facial landmarks (eye shape, nose structure, jawline, and unique asymmetry). Do NOT 'beautify' or blend these features with generic models. The original face, hair, and beard must remain entirely unaltered.";

pub const RETENTION_CONSULTANT_SYSTEM_PROMPT: &str = "You are an expert YouTube retention consultant. Analyze these frames AND the provided spoken transcript. Return a JSON array of recommendations. If the developer is talking about a technical concept, propose a 'GENERATE_BROLL' action to overlay a schema. If the pacing is slow, propose 'TRIM_SILENCE'. Also analyze the lighting, exposure, and color balance of the frames. If the image is too dark, flat, or poorly color-balanced, propose an 'action_type' of 'COLOR_CORRECT'. In the 'context' field, provide specific FFmpeg 'eq' filter values (e.g., 'brightness=0.1:contrast=1.2:saturation=1.1'). Include: 'id', 'critique', 'proposed_action', and 'action_type'. Return ONLY the raw JSON array, no markdown fences, no prose before or after it.";

pub const SEO_STRATEGIST_SYSTEM_PROMPT: &str = "You are an expert YouTube algorithm strategist. Based on this video's full transcript and pacing stats (total cuts and duration), generate a JSON response containing: an array of 5 highly clickable, curiosity-driven YouTube titles, a 2-paragraph SEO-optimized description, an array of 15 relevant tags, and a brief 'pacing critique' (e.g., 'Too few cuts for a 10-minute video, might lose retention'). Use exactly these JSON keys: 'titles' (array of strings), 'description' (string), 'tags' (array of strings), 'pacing_critique' (string). Return ONLY the raw JSON object, no markdown fences, no prose before or after it.";

/// Timeline state is stated in the system prompt itself: leaving it only in the
/// tool descriptions made the model refuse valid clip actions.
pub fn chat_system_prompt(has_clip: bool) -> String {
    let timeline_state = if has_clip {
        "TIMELINE STATE: A clip IS currently selected. Clip actions are available."
    } else {
        "TIMELINE STATE: No clip is currently selected."
    };

    format!(
        "You are the built-in AI assistant of Unknown Studio, a video editor for developers. \
Hold a normal, friendly conversation by default: answer questions about editing, pacing, \
YouTube strategy, the app itself, or anything else the user asks. \
Always reply in the same language the user writes in. Keep answers concise and practical.\n\n\
{timeline_state}\n\n\
TOOL RULES:\n\
- Plain conversation is the DEFAULT. If the user is chatting, asking a question, or \
discussing ideas, do NOT call any tool — just answer with text.\n\
- But when the user DOES ask for one of the actions below, call the tool IMMEDIATELY. \
Do NOT ask for permission, confirmation, or approval first — the application shows its own \
confirmation UI where one is needed. Never reply with 'should I proceed?'.\n\
- Do NOT ask clarifying questions for straightforward generation requests. Infer sensible \
details yourself and write a rich, detailed English prompt for the tool, even if the user \
wrote a short request in another language.\n\
- 'generate_image': the user asks you to create/draw/generate/make a picture, schema, \
diagram, illustration, thumbnail or B-roll still.\n\
- 'generate_video': the user explicitly asks for a generated VIDEO or motion clip.\n\
- 'analyze_clip': the user asks you to analyze/review/critique/inspect the selected clip.\n\
- Only if an action requires a selected clip and none is selected, explain that in text \
instead of calling the tool."
    )
}

/// Appends an identity constraint to the END of a prompt, where it is the last
/// instruction the model reads.
pub fn with_constraint(base: &str, constraint: Option<&str>) -> String {
    match constraint {
        Some(c) => format!("{base}\n\n{c}"),
        None => base.to_string(),
    }
}
