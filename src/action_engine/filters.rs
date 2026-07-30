//! Filter-spec sanitiser.
//!
//! The produced string is handed to `avfilter_graph_parse_ptr`, and its inputs
//! originate from user media fed through an LLM. Only numeric brightness /
//! contrast / saturation values are accepted and the chain is rebuilt here, so
//! model output can never inject arbitrary filters.

pub fn extract_eq_spec(raw: &str) -> Option<String> {
    let (mut brightness, mut contrast, mut saturation) = (None, None, None);

    for part in raw.split([':', ',', ';', '\n']) {
        // In fragments like `eq=brightness=0.1` the key precedes the last '='.
        let Some((key_part, value_part)) = part.rsplit_once('=') else {
            continue;
        };
        let Ok(value) = value_part.trim().parse::<f64>() else {
            continue;
        };
        let key = key_part
            .rsplit('=')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();

        match key.as_str() {
            "brightness" | "b" => brightness = Some(value.clamp(-1.0, 1.0)),
            "contrast" | "c" => contrast = Some(value.clamp(0.0, 3.0)),
            "saturation" | "s" => saturation = Some(value.clamp(0.0, 3.0)),
            _ => {}
        }
    }

    if brightness.is_none() && contrast.is_none() && saturation.is_none() {
        return None;
    }

    Some(format!(
        "eq=brightness={:.4}:contrast={:.4}:saturation={:.4}",
        brightness.unwrap_or(0.0),
        contrast.unwrap_or(1.0),
        saturation.unwrap_or(1.0),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_plain_values() {
        assert_eq!(
            extract_eq_spec("brightness=0.1:contrast=1.2:saturation=1.1").as_deref(),
            Some("eq=brightness=0.1000:contrast=1.2000:saturation=1.1000")
        );
    }

    #[test]
    fn extracts_values_with_eq_prefix() {
        assert_eq!(
            extract_eq_spec("eq=brightness=-0.2:contrast=0.9").as_deref(),
            Some("eq=brightness=-0.2000:contrast=0.9000:saturation=1.0000")
        );
    }

    #[test]
    fn rejects_injected_filters() {
        let spec = extract_eq_spec("brightness=0.1,movie=/etc/passwd,drawtext=text=pwned")
            .expect("known param should still parse");
        assert_eq!(spec, "eq=brightness=0.1000:contrast=1.0000:saturation=1.0000");
    }

    #[test]
    fn returns_none_without_numeric_params() {
        assert_eq!(extract_eq_spec("make it look cinematic"), None);
    }

    #[test]
    fn clamps_out_of_range_values() {
        assert_eq!(
            extract_eq_spec("brightness=99:contrast=-5:saturation=42").as_deref(),
            Some("eq=brightness=1.0000:contrast=0.0000:saturation=3.0000")
        );
    }
}
