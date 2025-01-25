use std::{collections::HashMap, fs, hash::Hash, io::{Cursor, Read}, path::{Path, PathBuf}, str::from_utf8};

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
    #[serde(rename = "type")]
    type_: String,
    url: String,
    time: String,
    #[serde(rename = "releaseTime")]
    release_time: String,
    sha1: String,
    #[serde(rename = "complianceLevel")]
    compliance_level: i32,
}
#[derive(Deserialize)]
struct LauncherMetaV2{
    versions: Vec<Version>,
}
#[derive(Deserialize)]
struct ClientMeta{
    downloads: ClientMetaDownloads,
}
#[derive(Deserialize)]
struct ClientMetaDownloads{
    client: ClientMetaDownload,
}
#[derive(Deserialize)]
struct ClientMetaDownload{
    sha1: String,
    size: i32,
    url: String,
}
#[derive(Deserialize,Serialize)]
struct VersionResult(pub HashMap<String, i32>);
#[derive(Deserialize)]
struct MCVersionData{
    data:i32
}
#[derive(Deserialize)]
struct MCVersionFile{
    pack_version: MCVersionData
}
#[tokio::main]
async fn main() {
    let response = reqwest::get("https://launchermeta.mojang.com/mc/game/version_manifest_v2.json").await.unwrap();
    let data = response.json::<LauncherMetaV2>().await.unwrap();
    let mut results = serde_json::from_str::<VersionResult>(
        fs::read_to_string("result.json").unwrap_or_else(|_|"{}".to_string()).as_str()
    ).unwrap().0;
    if results.len() == data.versions.len(){
        println!("All versions are already processed");
    }
    let mut idx = 0;
    let count = data.versions.len();
    let tasks = data.versions.iter().map(|version| {
        let id = version.id.clone();
        let url = version.url.clone();
        let exists = match results.get(&id) {
            Some(data) => Some(*data),
            None => None
        };
        async move {
            if let Some(data) = exists {
                return (id, data);
            }
            
            let client_meta = reqwest::get(url).await.unwrap().json::<ClientMeta>().await.unwrap();
            println!("GotClientMeta {}...", id);
            let contents = reqwest::get(client_meta.downloads.client.url.clone()).await.unwrap().bytes().await.unwrap();
            println!("GotClient {}...", id);
            let mut zip = zip::ZipArchive::new(Cursor::new(contents)).ok().unwrap();
            println!("Unzipped {}...", id);
            let data = match zip.by_name("version.json") {
                Ok(data) => data.bytes(),
                Err(_) => {
                    println!("version {} does not have a version.json", id);
                    return (id, -1);
                }
            };
            let data = serde_json::from_slice::<MCVersionFile>(data.into_iter().map(|f|f.unwrap()).collect::<Vec<_>>().as_slice());
            match data {
                Ok(data) => {
                    println!("version {} has a datapack version {}", id,data.pack_version.data);
                    return (id, data.pack_version.data);
                },
                Err(e) => {
                    println!("version {} does not have a datapack version", id);
                    return (id, -1);
                }
            }
        }
    }).collect::<Vec<_>>();
    let mut tokio_tasks = JoinSet::new();
    let mut i = 0;
    let concurrent_tasks = 3;
    for task in tasks {
        tokio_tasks.spawn(task);
        i += 1;
        if i >= concurrent_tasks {
            let result = tokio_tasks.join_next().await.unwrap().ok().unwrap();
            idx+=1;
            println!("Completed {} {}/{}...", result.0,idx,count);

            results.insert(result.0, result.1);
        }
        fs::write("result.json", serde_json::to_string_pretty(&VersionResult(results.clone())).unwrap()).unwrap();
    }
    let join_results = tokio_tasks.join_all().await;
    for (k,v) in join_results{
        results.insert(k, v);
    }
    fs::write("result.json", serde_json::to_string_pretty(&VersionResult(results)).unwrap()).unwrap();
}
