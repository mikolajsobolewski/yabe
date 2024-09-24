use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use log::debug;
use yaml_rust2::yaml::{Hash, Yaml};
use crate::deep_equal::deep_equal;

/// Recursively computes the difference between an override YAML object and the helm values YAML object.
pub fn compute_diff<'a>(obj: &'a Yaml, helm: &'a Yaml) -> Option<Cow<'a, Yaml>> {
    if deep_equal(obj, helm) {
        None
    } else {
        match (obj, helm) {
            (Yaml::Hash(obj_hash), Yaml::Hash(helm_hash)) => {
                let mut diff_hash = Hash::with_capacity(obj_hash.len());
                for (key, obj_value) in obj_hash {
                    let helm_value = helm_hash.get(key).unwrap_or(&Yaml::Null);
                    if let Some(diff_value) = compute_diff(obj_value, helm_value) {
                        diff_hash.insert(key.clone(), diff_value.into_owned());
                    }
                }
                if diff_hash.is_empty() {
                    None
                } else {
                    Some(Cow::Owned(Yaml::Hash(diff_hash)))
                }
            }
            (Yaml::Array(obj_array), Yaml::Array(helm_array)) => {
                if obj_array.len() != helm_array.len() {
                    Some(Cow::Borrowed(obj))
                } else {
                    let mut diffs = Vec::with_capacity(obj_array.len());
                    let mut has_diff = false;
                    for (obj_item, helm_item) in obj_array.iter().zip(helm_array.iter()) {
                        if let Some(diff_item) = compute_diff(obj_item, helm_item) {
                            diffs.push(diff_item.into_owned());
                            has_diff = true;
                        } else {
                            diffs.push(Yaml::Null);
                        }
                    }
                    if has_diff {
                        Some(Cow::Owned(Yaml::Array(diffs)))
                    } else {
                        None
                    }
                }
            }
            _ => Some(Cow::Borrowed(obj)),
        }
    }
}

/// Recursively computes the common base and differences among multiple Yaml objects.
pub fn diff_and_common_multiple<'a>(
    objs: &'a [&'a Yaml],
    quorum: f64,
) -> (Option<Cow<'a, Yaml>>, Vec<Option<Cow<'a, Yaml>>>) {

    debug!(
        "diff_and_common_multiple called with {} objects and quorum {}%.",
        objs.len(),
        quorum * 100.0
    );

    if objs.is_empty() {
        debug!("No objects to process. Returning.");
        return (None, vec![]);
    }

    let total_files = objs.len();
    let quorum_count = (quorum * total_files as f64).ceil() as usize;

    // Collect types of each object
    let types: Vec<&str> = objs
        .iter()
        .map(|obj| match obj {
            Yaml::Null => "null",
            Yaml::Boolean(_) => "bool",
            Yaml::Integer(_) => "int",
            Yaml::Real(_) => "real",
            Yaml::String(_) => "string",
            Yaml::Array(_) => "array",
            Yaml::Hash(_) => "hash",
            _ => "unknown",
        })
        .collect();

    let type_set: HashSet<&str> = types.iter().cloned().collect();

    // If types differ, include them in diffs
    if type_set.len() > 1 {
        debug!("Types differ. Including non-null values in diffs.");
        return (
            None,
            objs.iter()
                .map(|obj| if obj.is_null() { None } else { Some(Cow::Borrowed(*obj)) })
                .collect(),
        );
    }

    let obj_type = types[0];

    // Handle primitive types (non-object, non-array)
    if obj_type != "hash" && obj_type != "array" {
        debug!("Handling primitive types.");

        // Count occurrences of each value
        let mut occurrences = HashMap::new();
        for obj in objs {
            *occurrences.entry(obj).or_insert(0) += 1;
        }

        // Find the value(s) that meet the quorum
        let mut base_value = None;
        for (val, count) in occurrences {
            if count >= quorum_count {
                base_value = Some(val);
                break;
            }
        }

        if let Some(base_val) = base_value {
            debug!("Base value determined by quorum: {:?}", base_val);
            let diffs = objs
                .iter()
                .map(|obj| {
                    if deep_equal(obj, base_val) {
                        None
                    } else {
                        Some(Cow::Borrowed(*obj))
                    }
                })
                .collect();
            return (Some(Cow::Borrowed(base_val)), diffs);
        } else {
            // No value meets the quorum; include non-null values in diffs
            debug!("No value meets the quorum; including non-null values in diffs.");
            return (
                None,
                objs.iter()
                    .map(|obj| if obj.is_null() { None } else { Some(Cow::Borrowed(*obj)) })
                    .collect(),
            );
        }
    }

    // Handle hashes (maps)
    if obj_type == "hash" {
        debug!("Handling hashes (maps).");
        // Collect all unique keys
        let mut all_keys = HashSet::new();
        for obj in objs {
            if let Yaml::Hash(ref h) = obj {
                for key in h.keys() {
                    all_keys.insert(key);
                }
            }
        }

        // Initialize base hash and diffs
        let mut base_hash = Hash::new();
        let mut diffs: Vec<Hash> = vec![Hash::new(); objs.len()];
        let mut has_base = false;
        let mut has_diffs = vec![false; objs.len()];

        // Iterate over all keys
        for key in &all_keys {
            debug!("Processing key: {:?}", key);

            // Collect values at current key from all objects
            let values_at_key: Vec<&Yaml> = objs
                .iter()
                .map(|obj| {
                    if let Yaml::Hash(ref h) = obj {
                        h.get(*key).unwrap_or(&Yaml::Null)
                    } else {
                        &Yaml::Null
                    }
                })
                .collect();

            // Recursively process the values at this key
            let (sub_base, sub_diffs) = diff_and_common_multiple(&values_at_key, quorum);

            if let Some(sub_base_val) = sub_base {
                // Base value meets quorum
                base_hash.insert((*key).clone(), sub_base_val.into_owned());
                has_base = true;
            }

            for (i, sub_diff) in sub_diffs.into_iter().enumerate() {
                if let Some(sub_diff_val) = sub_diff {
                    // Only include non-null diffs
                    if !sub_diff_val.is_null() {
                        diffs[i].insert((*key).clone(), sub_diff_val.into_owned());
                        has_diffs[i] = true;
                    }
                }
            }
        }

        // Prepare base and diffs for return
        let base = if has_base {
            Some(Cow::Owned(Yaml::Hash(base_hash)))
        } else {
            None
        };

        let diffs_result: Vec<Option<Cow<'a, Yaml>>> = diffs
            .into_iter()
            .enumerate()
            .map(|(i, h)| {
                if has_diffs[i] {
                    Some(Cow::Owned(Yaml::Hash(h)))
                } else {
                    None
                }
            })
            .collect();

        return (base, diffs_result);
    }

    // Should not reach here; treat as diffs
    debug!("Unhandled object type. Including entire values in diffs.");
    (
        None,
        objs.iter().map(|obj| Some(Cow::Borrowed(*obj))).collect(),
    )
}