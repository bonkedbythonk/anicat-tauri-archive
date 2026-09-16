//! Intro and outro times from api.aniskip.com, keyed by MyAnimeList id and
//! episode: the Mac's `AniSkipClient`. Without them the skip button only
//! works on releases that chapter their OP and ED.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Segment {
    /// `op` or `ed`, the names the Lua script's chapter matcher uses.
    #[serde(rename = "type")]
    pub kind: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Deserialize)]
struct Response {
    found: bool,
    #[serde(default)]
    results: Vec<Found>,
}

#[derive(Deserialize)]
struct Found {
    interval: Interval,
    #[serde(rename = "skipType")]
    skip_type: String,
}

#[derive(Deserialize)]
struct Interval {
    #[serde(rename = "startTime")]
    start_time: f64,
    #[serde(rename = "endTime")]
    end_time: f64,
}

/// An empty list for "found nothing", the common case for anything not
/// popular enough to have community timestamps.
pub fn parse(body: &str) -> Result<Vec<Segment>, String> {
    let r: Response = serde_json::from_str(body).map_err(|e| e.to_string())?;
    if !r.found {
        return Ok(Vec::new());
    }
    Ok(r
        .results
        .into_iter()
        .filter(|f| matches!(f.skip_type.as_str(), "op" | "ed") && f.interval.end_time > f.interval.start_time)
        .map(|f| Segment { kind: f.skip_type, start: f.interval.start_time, end: f.interval.end_time })
        .collect())
}

/// `episodeLength` is the playing file's: AniSkip matches submissions made
/// against a file of that length, and a different encode can place the OP
/// elsewhere. Every failure comes back as `Err` for the log and nothing
/// else; skip times are a nicety, not something to show an error for.
pub async fn fetch(http: &reqwest::Client, mal_id: i64, episode: i64, length: f64) -> Result<Vec<Segment>, String> {
    let url = format!("https://api.aniskip.com/v2/skip-times/{mal_id}/{episode}");
    let length = format!("{}", length.round() as i64);
    let resp = http
        .get(&url)
        .query(&[("types", "op"), ("types", "ed"), ("episodeLength", length.as_str())])
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let body = resp.text().await.map_err(|e| e.to_string())?;
    // AniSkip answers "nothing for this episode" as a 404 with a normal body.
    if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
        parse(&body)
    } else {
        Err(format!("HTTP {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Recorded 2026-09-15: Frieren (MAL 52991) episode 1, episodeLength=1500.
    const FRIEREN_1: &str = r#"{"found":true,"results":[{"interval":{"startTime":116.748,"endTime":206.748},"skipType":"op","skipId":"657f5b16-f38c-4bb3-845f-ab6457956983","episodeLength":1510.092},{"interval":{"startTime":1417.135,"endTime":1507.135},"skipType":"ed","skipId":"92965d0e-bd0f-4753-a520-c2d6f2dffc47","episodeLength":1510.102}],"message":"Successfully found skip times","statusCode":200}"#;
    /// Recorded the same day for episode 2.
    const NOT_FOUND: &str = r#"{"found":false,"results":[],"message":"No skip times found","statusCode":404}"#;

    #[test]
    fn parses_op_and_ed() {
        let s = parse(FRIEREN_1).unwrap();
        assert_eq!(
            s,
            vec![
                Segment { kind: "op".into(), start: 116.748, end: 206.748 },
                Segment { kind: "ed".into(), start: 1417.135, end: 1507.135 },
            ]
        );
        let json = serde_json::to_value(&s[0]).unwrap();
        assert_eq!(json["type"], "op");
        assert_eq!(json["end"], 206.748);
    }

    #[test]
    fn not_found_is_empty_and_garbage_is_an_error() {
        assert!(parse(NOT_FOUND).unwrap().is_empty());
        assert!(parse("<html>").is_err());
    }
}
