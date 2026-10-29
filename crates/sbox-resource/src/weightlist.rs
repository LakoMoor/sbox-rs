//! Bone weight lists from the text `.vmdl_prefab` sources (`WeightList` nodes: name, default weight, per-bone weights).
//! The compiled model does not carry them, so this reads the source file shipped next to the assets.

use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct WeightList {
    pub default_weight: f32,
    pub weights: HashMap<String, f32>,
}

impl WeightList {
    /// Weights in the order of `bone_names` (unlisted bones get the default weight).
    pub fn per_bone(&self, bone_names: &[String]) -> Vec<f32> {
        bone_names.iter().map(|n| self.weights.get(n).copied().unwrap_or(self.default_weight)).collect()
    }
}

fn quoted(line: &str) -> Option<&str> {
    let a = line.find('"')? + 1;
    let b = line[a..].find('"')? + a;
    Some(&line[a..b])
}

fn number(line: &str) -> Option<f32> {
    line.split('=').nth(1)?.trim().trim_end_matches(',').parse().ok()
}

/// Parse every `WeightList` in a modeldoc text file.
pub fn parse(text: &str) -> HashMap<String, WeightList> {
    let mut out = HashMap::new();
    let mut cur: Option<(String, WeightList)> = None;
    let mut bone: Option<String> = None;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with("_class") {
            if let Some((n, wl)) = cur.take() {
                out.insert(n, wl);
            }
            bone = None;
            if quoted(l) == Some("WeightList") {
                cur = Some((String::new(), WeightList::default()));
            }
            continue;
        }
        let Some((name, wl)) = cur.as_mut() else { continue };
        if let Some(rest) = l.strip_prefix("name") {
            if name.is_empty() && rest.trim_start().starts_with('=') {
                *name = quoted(l).unwrap_or("").to_string();
            }
        } else if l.starts_with("default_weight") {
            wl.default_weight = number(l).unwrap_or(0.0);
        } else if l.starts_with("bone") {
            bone = quoted(l).map(String::from);
        } else if l.starts_with("weight") && !l.starts_with("weights") {
            if let (Some(b), Some(w)) = (bone.take(), number(l)) {
                wl.weights.insert(b, w);
            }
        }
    }
    if let Some((n, wl)) = cur {
        out.insert(n, wl);
    }
    out.retain(|n, _| !n.is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_names_defaults_and_bones() {
        let text = r#"
            {
                _class = "WeightList"
                name = "Demo"
                default_weight = 0.25
                weights =
                [
                    {
                        bone = "spine"
                        weight = 1.0
                    },
                    {
                        bone = "head"
                        weight = 0.5
                    },
                ]
            }
        "#;
        let m = parse(text);
        let w = &m["Demo"];
        assert_eq!(w.default_weight, 0.25);
        assert_eq!(w.per_bone(&["head".into(), "spine".into(), "foot".into()]), vec![0.5, 1.0, 0.25]);
    }

    #[test]
    fn citizen_lists_load() {
        let Ok(t) = std::fs::read_to_string("E:/sbox-ref/game/addons/citizen/Assets/models/citizen/prefabs/citizen_weightlistlist.vmdl_prefab") else { return };
        let m = parse(&t);
        assert!(m.len() >= 60, "{} lists", m.len());
        let up = &m["Only_UpperBody"];
        assert!(up.weights.values().any(|w| *w > 0.9), "upper body list has full-weight bones");
    }
}
