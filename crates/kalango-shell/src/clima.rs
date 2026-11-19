//! Weather for the "Hoje" bar (design "Clima"): now, the next hours and today's high and low, from Open-Meteo
//! (no account or key). It stays off until the person picks a city: only then does anything go out, and what goes
//! out is that city's coordinates. `KALANGO_CLIMA_URL` points both requests elsewhere (tests use a local server).

use kalango_i18n::{n_, tr};
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub struct City {
    pub name: String,
    pub region: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Weather {
    pub temp: f64,
    pub code: i64,
    /// (hour of day, temperature) for the next four 3-hour steps
    pub hours: Vec<(u32, f64)>,
    pub max: f64,
    pub min: f64,
}

fn config_file() -> PathBuf {
    gtk::glib::user_config_dir().join("kalango/clima.json")
}

/// The test server, when one is given (an empty value — the dev session passes the variable on — is none).
fn test_base() -> Option<String> {
    std::env::var("KALANGO_CLIMA_URL").ok().filter(|u| !u.trim().is_empty())
}

fn forecast_base() -> String {
    test_base().unwrap_or_else(|| "https://api.open-meteo.com".into())
}

fn geocoding_base() -> String {
    test_base().unwrap_or_else(|| "https://geocoding-api.open-meteo.com".into())
}

/// Brazil's states by their codes ("Maringá, PR").
const UF: &[(&str, &str)] = &[("ac", "Acre"), ("al", "Alagoas"), ("ap", "Amapá"), ("am", "Amazonas"), ("ba", "Bahia"), ("ce", "Ceará"),
    ("df", "Distrito Federal"), ("es", "Espírito Santo"), ("go", "Goiás"), ("ma", "Maranhão"), ("mt", "Mato Grosso"), ("ms", "Mato Grosso do Sul"),
    ("mg", "Minas Gerais"), ("pa", "Pará"), ("pb", "Paraíba"), ("pr", "Paraná"), ("pe", "Pernambuco"), ("pi", "Piauí"), ("rj", "Rio de Janeiro"),
    ("rn", "Rio Grande do Norte"), ("rs", "Rio Grande do Sul"), ("ro", "Rondônia"), ("rr", "Roraima"), ("sc", "Santa Catarina"), ("sp", "São Paulo"),
    ("se", "Sergipe"), ("to", "Tocantins")];

/// Lower case without accents ("São Paulo" → "sao paulo").
fn fold(s: &str) -> String {
    s.to_lowercase().chars().map(|c| match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' => 'a', 'é' | 'è' | 'ê' | 'ë' => 'e', 'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o', 'ú' | 'ù' | 'û' | 'ü' => 'u', 'ç' => 'c', other => other,
    }).collect()
}

/// "maringá,pr" → ("maringá", Some("pr")): the name is searched; what comes after the comma (a state, its code, a
/// country) chooses among the cities found.
pub(crate) fn split_place(q: &str) -> (String, Option<String>) {
    match q.split_once(',') {
        Some((a, b)) if !b.trim().is_empty() => (a.trim().to_string(), Some(b.trim().to_string())),
        _ => (q.trim().trim_end_matches(',').to_string(), None),
    }
}

/// Whether a city's state or country is the one written after the comma.
pub(crate) fn place_matches(hint: &str, admin1: &str, country: &str, country_code: &str) -> bool {
    let h = fold(hint);
    let state = UF.iter().find(|(c, _)| *c == h).map(|(_, n)| fold(n)).unwrap_or_else(|| h.clone());
    fold(admin1) == state || fold(admin1).starts_with(&h) || fold(country).starts_with(&h) || fold(country_code) == h
}

fn get(url: &str) -> Result<Value, String> {
    ureq::get(url).timeout(Duration::from_secs(10)).call().map_err(|e| e.to_string())?.into_json::<Value>().map_err(|e| e.to_string())
}

/// The city the person chose (None: weather is off).
pub fn city() -> Option<City> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(config_file()).ok()?).ok()?;
    Some(City { name: v["cidade"].as_str()?.to_string(), region: v["regiao"].as_str().unwrap_or("").to_string(), lat: v["lat"].as_f64()?, lon: v["lon"].as_f64()? })
}

pub fn set_city(c: Option<&City>) -> Result<(), String> {
    let f = config_file();
    match c {
        None => {
            let _ = std::fs::remove_file(&f);
            Ok(())
        }
        Some(c) => {
            if let Some(d) = f.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            let v = serde_json::json!({"cidade": c.name, "regiao": c.region, "lat": c.lat, "lon": c.lon});
            std::fs::write(&f, serde_json::to_string_pretty(&v).unwrap_or_default()).map_err(|e| e.to_string())
        }
    }
}

/// Cities matching a name (blocking: call off the UI thread).
pub fn search(name: &str, language: &str) -> Result<Vec<City>, String> {
    let (name, hint) = split_place(name);
    let url = format!("{}/v1/search?count=10&language={language}&format=json&name={}", geocoding_base(), urlencode(&name));
    let v = get(&url)?;
    let all: Vec<(City, bool)> = v["results"].as_array().into_iter().flatten().filter_map(|r| {
        let (a1, co, cc) = (r["admin1"].as_str().unwrap_or(""), r["country"].as_str().unwrap_or(""), r["country_code"].as_str().unwrap_or(""));
        let region = [a1, co].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
        let hit = hint.as_deref().is_some_and(|h| place_matches(h, a1, co, cc));
        Some((City { name: r["name"].as_str()?.to_string(), region, lat: r["latitude"].as_f64()?, lon: r["longitude"].as_f64()? }, hit))
    }).collect();
    // (with a state or country after the comma: only those, when any matches)
    let any = all.iter().any(|(_, h)| *h);
    Ok(all.into_iter().filter(|(_, h)| !any || *h).map(|(c, _)| c).take(5).collect())
}

/// The forecast for the city (blocking: call off the UI thread).
pub fn forecast(c: &City) -> Result<Weather, String> {
    let url = format!("{}/v1/forecast?latitude={:.3}&longitude={:.3}&current=temperature_2m,weather_code&hourly=temperature_2m&daily=temperature_2m_max,temperature_2m_min&timezone=auto&forecast_days=2",
        forecast_base(), c.lat, c.lon);
    parse(&get(&url)?)
}

pub(crate) fn parse(v: &Value) -> Result<Weather, String> {
    let temp = v["current"]["temperature_2m"].as_f64().ok_or("no current temperature")?;
    let code = v["current"]["weather_code"].as_i64().unwrap_or(0);
    let now = v["current"]["time"].as_str().unwrap_or("");
    let times: Vec<&str> = v["hourly"]["time"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let temps: Vec<f64> = v["hourly"]["temperature_2m"].as_array().into_iter().flatten().filter_map(Value::as_f64).collect();
    // the next four 3-hour steps after now (hours like "2026-11-16T03:00")
    let start = times.iter().position(|t| *t > now).unwrap_or(0);
    let mut hours = Vec::new();
    let mut i = start;
    while hours.len() < 4 && i < times.len().min(temps.len()) {
        let h: u32 = times[i].get(11..13).and_then(|x| x.parse().ok()).unwrap_or(0);
        if h.is_multiple_of(3) {
            hours.push((h, temps[i]));
        }
        i += 1;
    }
    let max = v["daily"]["temperature_2m_max"][0].as_f64().unwrap_or(temp);
    let min = v["daily"]["temperature_2m_min"][0].as_f64().unwrap_or(temp);
    Ok(Weather { temp, code, hours, max, min })
}

/// The WMO weather code in words.
pub fn describe(code: i64) -> &'static str {
    tr(match code {
        0 => n_("clear sky"),
        1 | 2 => n_("partly cloudy"),
        3 => n_("cloudy"),
        45 | 48 => n_("fog"),
        51..=57 => n_("drizzle"),
        61..=67 | 80..=82 => n_("rain"),
        71..=77 | 85 | 86 => n_("snow"),
        95..=99 => n_("thunderstorm"),
        _ => n_("weather"),
    })
}

fn urlencode(s: &str) -> String {
    s.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
        _ => format!("%{b:02X}"),
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_forecast_has_now_the_next_hours_and_the_day() {
        let v = serde_json::json!({
            "current": {"time": "2026-11-16T02:15", "temperature_2m": 19.4, "weather_code": 0},
            "hourly": {"time": ["2026-11-16T02:00", "2026-11-16T03:00", "2026-11-16T04:00", "2026-11-16T05:00", "2026-11-16T06:00",
                "2026-11-16T07:00", "2026-11-16T08:00", "2026-11-16T09:00", "2026-11-16T10:00", "2026-11-16T11:00", "2026-11-16T12:00"],
                "temperature_2m": [19.0, 18.2, 18.0, 17.5, 17.1, 18.0, 19.5, 21.3, 23.0, 24.8, 26.1]},
            "daily": {"temperature_2m_max": [27.2, 28.0], "temperature_2m_min": [16.1, 17.0]}
        });
        let w = parse(&v).unwrap();
        assert_eq!(w.hours.iter().map(|h| h.0).collect::<Vec<_>>(), vec![3, 6, 9, 12]);
        assert_eq!((w.temp.round(), w.max.round(), w.min.round()), (19.0, 27.0, 16.0));
        assert_eq!(urlencode("São Paulo"), "S%C3%A3o%20Paulo");
    }
}

#[cfg(test)]
mod tests_lugar {
    use super::{place_matches, split_place};

    #[test]
    fn city_with_state() {
        assert_eq!(split_place("maringá,pr"), ("maringá".to_string(), Some("pr".to_string())));
        assert_eq!(split_place("Lisboa"), ("Lisboa".to_string(), None));
        assert!(place_matches("pr", "Paraná", "Brasil", "BR"));
        assert!(!place_matches("pr", "Rio Grande do Sul", "Brasil", "BR"));
        assert!(place_matches("Portugal", "Lisboa", "Portugal", "PT"));
    }
}
