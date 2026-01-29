use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Read},
};

use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;

#[derive(Deserialize)]
struct Version {
    /*
    {
      "id": "rd-132211",
      "type": "old_alpha",
      "url": "https://piston-meta.mojang.com/v1/packages/d090f5d3766a28425316473d9ab6c37234d48b02/rd-132211.json",
      "time": "2022-03-10T09:51:38+00:00",
      "releaseTime": "2009-05-13T20:11:00+00:00",
      "sha1": "d090f5d3766a28425316473d9ab6c37234d48b02",
      "complianceLevel": 0
    }
     */
    id: String,
    // #[serde(rename = "type")]
    // type_: String,
    url: String,
    // time: String,
    // #[serde(rename = "releaseTime")]
    // release_time: String,
    // sha1: String,
    // #[serde(rename = "complianceLevel")]
    // compliance_level: i32,
}
#[derive(Deserialize)]
struct LauncherMetaV2 {
    versions: Vec<Version>,
}
#[derive(Deserialize)]
struct ClientMeta {
    downloads: ClientMetaDownloads,
}
#[derive(Deserialize)]
struct ClientMetaDownloads {
    client: ClientMetaDownload,
}
#[derive(Deserialize)]
struct ClientMetaDownload {
    // pub sha1: String,
    // pub size: i32,
    pub url: String,
}
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(untagged)]
enum PackVersion {
    Single(i32),
    MajorMinor(i32, i32),
}

#[derive(Deserialize, Serialize)]
struct VersionResult(pub BTreeMap<String, PackVersion>);

// Old format: { "data": 123 }
#[derive(Deserialize)]
struct MCVersionDataOld {
    data: i32,
}

// New format: { "data_major": 98, "data_minor": 0, ... }
#[derive(Deserialize)]
struct MCVersionDataNew {
    data_major: i32,
    data_minor: i32,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum MCVersionData {
    Old(MCVersionDataOld),
    New(MCVersionDataNew),
}

#[derive(Deserialize)]
struct MCVersionFile {
    pack_version: MCVersionData,
}
#[tokio::main]
async fn main() {
    let response = reqwest::get("https://launchermeta.mojang.com/mc/game/version_manifest_v2.json")
        .await
        .unwrap();
    let data = response.json::<LauncherMetaV2>().await.unwrap();
    let mut results = serde_json::from_str::<VersionResult>(
        fs::read_to_string("result.json")
            .unwrap_or_else(|_| "{}".to_string())
            .as_str(),
    )
    .unwrap()
    .0;
    if results.len() == data.versions.len() {
        println!("All versions are already processed");
    }
    let mut idx = 0;
    let count = data.versions.len();
    let tasks = data
        .versions
        .iter()
        .map(|version| {
            let id = version.id.clone();
            let url = version.url.clone();
            let exists = results.get(&id).cloned();
            async move {
                if let Some(data) = exists {
                    return (id, data);
                }

                let client_meta = reqwest::get(url)
                    .await
                    .unwrap()
                    .json::<ClientMeta>()
                    .await
                    .unwrap();
                println!("GotClientMeta {}...", id);
                let contents = reqwest::get(client_meta.downloads.client.url.clone())
                    .await
                    .unwrap()
                    .bytes()
                    .await
                    .unwrap();
                println!("GotClient {}...", id);
                let mut zip = zip::ZipArchive::new(Cursor::new(contents)).ok().unwrap();
                println!("Unzipped {}...", id);
                let data = match zip.by_name("version.json") {
                    Ok(data) => data.bytes(),
                    Err(_) => {
                        println!("version {} does not have a version.json", id);
                        return (id, PackVersion::Single(-1));
                    }
                };
                let data = serde_json::from_slice::<MCVersionFile>(
                    data.into_iter()
                        .map(|f| f.unwrap())
                        .collect::<Vec<_>>()
                        .as_slice(),
                );
                match data {
                    Ok(data) => match data.pack_version {
                        MCVersionData::Old(old) => {
                            println!("version {} has a datapack version {}", id, old.data);
                            (id, PackVersion::Single(old.data))
                        }
                        MCVersionData::New(new) => {
                            println!(
                                "version {} has a datapack version [{}, {}]",
                                id, new.data_major, new.data_minor
                            );
                            (id, PackVersion::MajorMinor(new.data_major, new.data_minor))
                        }
                    },
                    Err(_e) => {
                        println!("version {} does not have a datapack version", id);
                        (id, PackVersion::Single(-1))
                    }
                }
            }
        })
        .collect::<Vec<_>>();
    let mut tokio_tasks = JoinSet::new();
    let mut i = 0;
    let concurrent_tasks = 3;
    for task in tasks {
        tokio_tasks.spawn(task);
        i += 1;
        if i >= concurrent_tasks {
            let result = tokio_tasks.join_next().await.unwrap().ok().unwrap();
            idx += 1;
            println!("Completed {} {}/{}...", result.0, idx, count);

            results.insert(result.0, result.1);
        }
        fs::write(
            "result.json",
            serde_json::to_string_pretty(&VersionResult(results.clone())).unwrap(),
        )
        .unwrap();
    }
    let join_results = tokio_tasks.join_all().await;
    for (k, v) in join_results {
        results.insert(k, v);
    }
    fs::write(
        "result.json",
        serde_json::to_string_pretty(&VersionResult(results)).unwrap(),
    )
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pack_version_serialization() {
        let mut results = BTreeMap::new();

        // Old format: single integer
        results.insert("1.0".to_string(), PackVersion::Single(-1));
        results.insert("1.13".to_string(), PackVersion::Single(4));

        // New format: tuple of [data_major, data_minor]
        results.insert("26.1-snapshot-5".to_string(), PackVersion::MajorMinor(98, 0));

        let json = serde_json::to_string_pretty(&VersionResult(results.clone())).unwrap();
        println!("Generated JSON:\n{}", json);

        // Verify that the JSON contains the expected values
        assert!(json.contains("\"1.0\": -1"));
        assert!(json.contains("\"1.13\": 4"));
        assert!(json.contains("\"26.1-snapshot-5\": [\n    98,\n    0\n  ]"));

        // Test deserialization
        let parsed: VersionResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.0.len(), 3);

        match parsed.0.get("1.0").unwrap() {
            PackVersion::Single(v) => assert_eq!(*v, -1),
            _ => panic!("Expected Single variant"),
        }

        match parsed.0.get("26.1-snapshot-5").unwrap() {
            PackVersion::MajorMinor(major, minor) => {
                assert_eq!(*major, 98);
                assert_eq!(*minor, 0);
            }
            _ => panic!("Expected MajorMinor variant"),
        }
    }

    #[test]
    fn test_backward_compatibility() {
        // Test that existing result.json format can be deserialized
        let old_format_json = r#"{
  "1.0": -1,
  "1.13": 4,
  "1.14": 5
}"#;

        let parsed: VersionResult = serde_json::from_str(old_format_json).unwrap();
        assert_eq!(parsed.0.len(), 3);

        match parsed.0.get("1.0").unwrap() {
            PackVersion::Single(v) => assert_eq!(*v, -1),
            _ => panic!("Expected Single variant for backward compatibility"),
        }

        match parsed.0.get("1.13").unwrap() {
            PackVersion::Single(v) => assert_eq!(*v, 4),
            _ => panic!("Expected Single variant for backward compatibility"),
        }
    }

    #[test]
    fn test_new_format_deserialization() {
        // Test that new array format can be deserialized
        let new_format_json = r#"{
  "26.1-snapshot-5": [98, 0],
  "26.2-snapshot-1": [99, 1]
}"#;

        let parsed: VersionResult = serde_json::from_str(new_format_json).unwrap();
        assert_eq!(parsed.0.len(), 2);

        match parsed.0.get("26.1-snapshot-5").unwrap() {
            PackVersion::MajorMinor(major, minor) => {
                assert_eq!(*major, 98);
                assert_eq!(*minor, 0);
            }
            _ => panic!("Expected MajorMinor variant for new format"),
        }
    }

    #[test]
    fn test_mc_version_data_old_format() {
        // Test parsing old pack_version format
        let old_json = r#"{"pack_version": {"data": 4}}"#;
        let parsed: MCVersionFile = serde_json::from_str(old_json).unwrap();

        match parsed.pack_version {
            MCVersionData::Old(old) => assert_eq!(old.data, 4),
            _ => panic!("Expected Old variant"),
        }
    }

    #[test]
    fn test_mc_version_data_new_format() {
        // Test parsing new pack_version format
        let new_json = r#"{"pack_version": {"data_major": 98, "data_minor": 0, "resource_major": 79, "resource_minor": 0}}"#;
        let parsed: MCVersionFile = serde_json::from_str(new_json).unwrap();

        match parsed.pack_version {
            MCVersionData::New(new) => {
                assert_eq!(new.data_major, 98);
                assert_eq!(new.data_minor, 0);
            }
            _ => panic!("Expected New variant"),
        }
    }
}
