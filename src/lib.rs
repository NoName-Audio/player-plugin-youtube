mod service;
use player_plugin_sdk::{Host, NativePlugin, Result};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Default)]
pub struct Plugin {
    service: service::Service,
    runtime: PathBuf,
    root: PathBuf,
    serial: u64,
    searches: BTreeMap<String, std::result::Result<Vec<String>, String>>,
    results: BTreeMap<String, Value>,
    pending: BTreeSet<String>,
    cover_pending: BTreeSet<String>,
    cover_attempted: BTreeSet<String>,
    previews: BTreeSet<String>,
    saves: BTreeSet<String>,
    tokens: BTreeMap<String, Value>,
    paths: BTreeMap<String, PathBuf>,
    completed: BTreeSet<String>,
    preview_started: BTreeSet<String>,
    states: BTreeMap<String, String>,
    playback: Value,
    error: String,
    notice: String,
}
fn text(v: &Value, key: &str) -> String { v[key].as_str().unwrap_or("").to_owned() }
impl Plugin {
    fn executable(&self) -> String {
        self.runtime.join(if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" }).to_string_lossy().into_owned()
    }
    fn playing(&self, id: &str) -> bool {
        self.playback["resource"]["pluginId"] == "player.youtube" && self.playback["resource"]["resourceId"] == id
    }
    fn row(&self, id: &str) -> Value {
        let r = &self.results[id];
        json!({"id":id,"title":r["title"],"subtitle":r["artist"],"coverHash":r["coverHash"],"coverLoading":self.cover_pending.contains(id),"status":self.states.get(id),"actions":[
            {"id":"play","label":if self.previews.contains(id){"Cancel preview"}else if self.playing(id) || self.preview_started.contains(id){"Stop preview"}else{"Preview"}},
            {"id":"save","label":if self.saves.contains(id){"Cancel save"}else{"Save to library"}}]})
    }
    fn search_rows(&self, key: &str) -> Vec<Value> {
        match self.searches.get(key) {
            Some(Ok(ids)) => ids.iter().map(|id| self.row(id)).collect(),
            _ => Vec::new(),
        }
    }
    fn search_response(&self, key: &str) -> Value {
        let error = self.searches.get(key).and_then(|result| result.as_ref().err());
        json!({"items":self.search_rows(key),"searchId":key,"pending":!self.searches.contains_key(key),"error":error,"nextCursor":null})
    }
    fn resource(&self, id: &str) -> Value {
        let r = &self.results[id];
        json!({"pluginId":"player.youtube","resourceId":id,"title":r["title"],"artist":r["artist"],"album":"","durationSeconds":r["duration"],"coverHash":r["coverHash"]})
    }
    fn clear_intents(&mut self, id: &str) {
        self.previews.remove(id); self.saves.remove(id); self.pending.remove(id);
    }
    fn cancel_unused(&mut self, id: &str, host: &dyn Host) -> Result {
        if !self.previews.contains(id) && !self.preview_started.contains(id) && !self.saves.contains(id) && self.pending.remove(id) {
            self.service.cancel(id);
            if let Some(path) = self.paths.remove(id) { host.call("media.state",json!({"path":path,"state":"cancelled"}))?; }
            self.tokens.remove(id); self.completed.remove(id); self.preview_started.remove(id);
            self.states.insert(id.to_owned(),"Cancelled".into());
        }
        Ok(Value::Null)
    }
    fn fulfill(&mut self, id: &str, host: &dyn Host) -> Result {
        let token = self.tokens.get(id).ok_or("Downloaded source unavailable")?.clone();
        let resource = self.resource(id);
        let play = self.previews.remove(id);
        let save = self.completed.contains(id) && self.saves.remove(id);
        let mut failure = None;
        let mut started = false;
        if play {
            match host.call("host.play",json!({"sourceToken":token,"resource":resource})) {
                Ok(_) => { started = true; self.preview_started.insert(id.to_owned()); }
                Err(e) => failure = Some(e),
            }
        }
        if save {
            match host.call("library.import",json!({"sourceToken":token,"coverHash":resource["coverHash"]})) {
                Ok(track) if !track.is_null() => {
                    self.notice = "Saved to your library.".into();
                    if started || self.preview_started.contains(id) || self.playing(id) {
                        if let Err(e) = host.call("host.adopt",json!({"resourceId":id,"track":track})) { failure = Some(e); }
                    }
                }
                Ok(_) => failure = Some("The downloaded file could not be indexed".into()),
                Err(e) => failure = Some(e),
            }
        }
        if let Some(e) = failure { return Err(e); }
        Ok(Value::Null)
    }
    fn event(&mut self, event: service::Event, host: &dyn Host) -> Result {
        match event {
            service::Event::Search { key, result } => {
                // Keys belong to a particular submission, never to the latest query.
                let result = result.map(|rows| rows.into_iter().map(|mut r| {
                    let id = text(&r,"id");
                    if let Some(previous) = self.results.get(&id) { r["coverHash"] = previous["coverHash"].clone(); }
                    if !r["thumbnail"].is_null() && self.cover_attempted.insert(id.clone()) {
                        if self.service.cover(id.clone(),text(&r,"url"),self.executable(),self.root.clone()) {
                            self.cover_pending.insert(id.clone());
                        } else { self.cover_attempted.remove(&id); }
                    }
                    self.results.insert(id.clone(),r); id
                }).collect());
                self.searches.insert(key,result);
            }
            service::Event::Cover { id, result } => {
                self.cover_pending.remove(&id);
                if let Ok(path) = result {
                    let hash = host.call("media.cover",json!({"path":path})).unwrap_or(Value::Null);
                    if let Some(row) = self.results.get_mut(&id) { row["coverHash"] = hash; }
                    if let Some(parent) = path.parent() { let _ = std::fs::remove_dir_all(parent); }
                }
            }
            service::Event::Progress { id, path } => {
                if !self.pending.contains(&id) { return Ok(Value::Null); }
                if !self.tokens.contains_key(&id) {
                    let token = host.call("media.register",json!({"path":path,"state":"active","resource":self.resource(&id)}))?;
                    self.paths.insert(id.clone(),path);
                    self.tokens.insert(id.clone(),token);
                }
                self.fulfill(&id,host)?;
            }
            service::Event::Download { id, result } => {
                if !self.pending.remove(&id) { return Ok(Value::Null); }
                match result {
                    Ok(path) => {
                        if let Some(active) = self.paths.get(&id) {
                            host.call("media.state",json!({"path":active,"state":"completed"}))?;
                        } else {
                            let token = host.call("media.register",json!({"path":path,"state":"completed","resource":self.resource(&id)}))?;
                            self.paths.insert(id.clone(),path);
                            self.tokens.insert(id.clone(),token);
                        }
                        self.completed.insert(id.clone());
                        self.states.insert(id.clone(),"Download complete".into());
                        self.fulfill(&id,host)?;
                    }
                    Err(e) => {
                        if let Some(path) = self.paths.remove(&id) { host.call("media.state",json!({"path":path,"state":"failed"}))?; }
                        self.tokens.remove(&id); self.completed.remove(&id); self.preview_started.remove(&id);
                        self.clear_intents(&id); self.states.insert(id,e.clone()); return Err(e);
                    }
                }
            }
        }
        Ok(Value::Null)
    }
    fn action(&mut self, args: Value, host: &dyn Host) -> Result {
        let action = text(&args,"action");
        if action != "play" && action != "save" { return Err("Unknown action".into()); }
        let id = text(&args["payload"],"resourceId");
        if !self.results.contains_key(&id) { return Err("Search again to restore this result".into()); }
        if action == "play" && (self.playing(&id) || self.preview_started.contains(&id)) {
            host.call("host.transport",json!({"action":"stop"}))?;
            self.preview_started.remove(&id); self.previews.remove(&id);
            self.cancel_unused(&id,host)?;
            return Ok(Value::Null);
        }
        let removed = if action == "play" { self.previews.remove(&id) } else { self.saves.remove(&id) };
        if removed {
            self.cancel_unused(&id,host)?;
            return Ok(Value::Null);
        }
        if action == "play" {
            let old: BTreeSet<_> = self.previews.union(&self.preview_started).cloned().collect();
            for old in old {
                if self.preview_started.contains(&old) || self.playing(&old) {
                    host.call("host.transport",json!({"action":"stop"}))?;
                }
                self.previews.remove(&old);
                self.preview_started.remove(&old);
                self.cancel_unused(&old,host)?;
            }
            self.previews.insert(id.clone());
        } else { self.saves.insert(id.clone()); }
        if self.tokens.contains_key(&id) { return self.fulfill(&id,host); }
        if self.pending.contains(&id) { return Ok(Value::Null); }
        let url = text(&self.results[&id],"url");
        if let Err(e) = self.service.download(id.clone(),url,self.executable(),self.root.clone()) {
            self.clear_intents(&id); self.states.insert(id,e.clone()); return Err(e);
        }
        self.pending.insert(id.clone()); self.states.insert(id,"Downloading…".into());
        Ok(Value::Null)
    }
    fn handle(&mut self, method: &str, args: Value, host: &dyn Host) -> Result {
        match method {
            "activate" => {
                let runtime = host.call("runtime.directory",json!({}))?;
                self.runtime = PathBuf::from(runtime["path"].as_str().ok_or("Invalid runtime directory")?);
                if !self.runtime.is_absolute() { return Err("Runtime directory must be absolute".into()); }
                for name in if cfg!(windows) { ["yt-dlp.exe", "deno.exe"] } else { ["yt-dlp", "deno"] } {
                    if !self.runtime.join(name).is_file() { return Err(format!("Bundled runtime missing: {name}; reinstall the YouTube plugin")); }
                }
                let root = host.call("storage.staging",json!({}))?;
                self.root = PathBuf::from(root["path"].as_str().ok_or("Invalid staging directory")?);
                Ok(Value::Null)
            }
            "deactivate" => {
                let pending: Vec<_> = self.pending.iter().cloned().collect();
                self.previews.clear(); self.preview_started.clear(); self.saves.clear();
                for id in pending { self.cancel_unused(&id,host)?; }
                self.service.stop(); self.cover_pending.clear(); self.cover_attempted.clear();
                Ok(Value::Null)
            }
            "playback" => { self.playback = args; host.invalidate(); Ok(Value::Null) }
            "poll" => {
                let events = self.service.drain();
                if events.is_empty() { return Ok(Value::Null); }
                for event in events { if let Err(e) = self.event(event,host) { self.error = e; } }
                host.invalidate(); Ok(Value::Null)
            }
            "search" => {
                self.serial += 1;
                let key = format!("youtube-search:{}",self.serial);
                self.service.search(key.clone(),text(&args,"query"),self.executable())?;
                Ok(self.search_response(&key))
            }
            "renderSearch" => {
                let ids: Vec<_> = args["ids"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                if let Some(key) = args["searchId"].as_str() { return Ok(self.search_response(key)); }
                Ok(json!(ids.into_iter().filter(|id| self.results.contains_key(*id)).map(|id| self.row(id)).collect::<Vec<_>>()))
            }
            "action" => { self.error.clear(); self.notice.clear(); let result = self.action(args,host); host.invalidate(); result }
            "render" => {
                let items: Vec<_> = self.states.keys().filter(|id| self.results.contains_key(*id)).map(|id| self.row(id)).collect();
                if items.is_empty() && self.error.is_empty() { return Ok(Value::Null); }
                Ok(json!({"type":"status","label":if self.error.is_empty(){"YouTube downloads"}else{&self.error},"tone":if !self.error.is_empty(){"error"}else if self.pending.is_empty(){"success"}else{"active"},"title":"YouTube","children":[{"type":"text","text":self.notice},{"type":"results","items":items}]}))
            }
            _ => Err("METHOD_NOT_FOUND".into()),
        }
    }
}
impl NativePlugin for Plugin {
    fn call(&mut self, method: &str, args: Value, host: &dyn Host) -> Result {
        let result = self.handle(method,args,host);
        if let Err(e) = &result { self.error = e.clone(); host.invalidate(); }
        result
    }
}
#[cfg(feature = "ffi")]
player_plugin_sdk::export_plugin!(Plugin,"player.youtube");

#[cfg(test)]
mod tests {
    use super::*;
    use player_plugin_sdk::testing::TestHost;
    #[test]
    fn covers_propagate_and_failures_clear_loading() {
        let mut p = ready();
        p.cover_pending.insert("video".into());
        assert_eq!(p.row("video")["coverLoading"],true);
        let host = TestHost::new(|method,_| match method {
            "media.cover" => Ok(json!("thumbnail-hash")),
            "media.register" => Ok(json!("source")),
            "library.import" => Ok(json!({"id":7})),
            _ => Ok(Value::Null),
        });
        p.event(service::Event::Cover { id:"video".into(), result:Ok(PathBuf::from("/nonexistent-youtube-fixture/cover.webp")) },&host).unwrap();
        assert_eq!(p.row("video")["coverHash"],"thumbnail-hash");
        assert_eq!(p.row("video")["coverLoading"],false);
        p.pending.insert("video".into()); p.previews.insert("video".into()); p.saves.insert("video".into());
        p.event(service::Event::Download { id:"video".into(), result:Ok(PathBuf::from("audio.m4a")) },&host).unwrap();
        for method in ["media.register","host.play"] {
            assert!(host.calls.borrow().iter().any(|(name,args)| name == method && args["resource"]["coverHash"] == "thumbnail-hash"));
        }
        assert!(host.calls.borrow().iter().any(|(name,args)| name == "library.import" && args["coverHash"] == "thumbnail-hash"));
        p.cover_pending.insert("video".into());
        p.event(service::Event::Cover { id:"video".into(), result:Err("unavailable".into()) },&host).unwrap();
        assert_eq!(p.row("video")["coverLoading"],false);
    }
    fn ready() -> Plugin {
        let mut p = Plugin::default();
        p.results.insert("video".into(),json!({"id":"video","title":"Song","artist":"Artist","duration":12,"url":"https://www.youtube.com/watch?v=video"}));
        p.tokens.insert("video".into(),json!("source")); p.completed.insert("video".into()); p
    }
    #[test]
    fn activation_resolves_only_packaged_executables() {
        let root = std::env::temp_dir().join(format!("youtube-activation-{}",std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        for name in if cfg!(windows) { ["yt-dlp.exe", "deno.exe"] } else { ["yt-dlp", "deno"] } {
            std::fs::write(root.join(name),b"fixture").unwrap();
        }
        let directory = root.clone();
        let host = TestHost::new(move |method,_| match method {
            "runtime.directory" | "storage.staging" => Ok(json!({"path":directory})),
            _ => Err(format!("Unexpected service: {method}")),
        });
        let mut plugin = Plugin::default();
        plugin.call("activate",json!({}),&host).unwrap();
        assert_eq!(PathBuf::from(plugin.executable()).parent(),Some(root.as_path()));
        assert_eq!(host.count("settings.get"),0);
        std::fs::remove_file(root.join(if cfg!(windows) { "deno.exe" } else { "deno" })).unwrap();
        assert!(plugin.call("activate",json!({}),&host).unwrap_err().contains("Bundled runtime missing"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn empty_searches_have_identity_without_fake_rows() {
        let mut p = Plugin::default();
        assert!(p.search_rows("youtube-search:1").is_empty());
        assert_eq!(p.search_response("youtube-search:1")["pending"],true);
        p.searches.insert("youtube-search:1".into(),Ok(vec![]));
        assert_eq!(p.search_response("youtube-search:1")["searchId"],"youtube-search:1");
        assert_eq!(p.search_response("youtube-search:1")["pending"],false);
        assert!(p.search_rows("youtube-search:1").is_empty());
    }
    #[test]
    fn render_search_refreshes_empty_ids_by_submission_and_keeps_legacy_arrays() {
        let mut p = ready(); let host = TestHost::new(|_,_| Ok(Value::Null));
        p.searches.insert("submission:1".into(),Ok(vec!["video".into()]));
        let refreshed = p.handle("renderSearch",json!({"searchId":"submission:1","ids":[]}),&host).unwrap();
        assert_eq!(refreshed["items"][0]["id"],"video");
        assert_eq!(refreshed["searchId"],"submission:1");
        assert_eq!(refreshed["pending"],false);
        assert!(p.handle("renderSearch",json!({"ids":["video"]}),&host).unwrap().is_array());
    }
    #[test]
    fn streaming_preview_starts_once_and_save_waits_for_completion() {
        let mut p = ready(); p.tokens.clear(); p.completed.clear();
        p.pending.insert("video".into()); p.previews.insert("video".into()); p.saves.insert("video".into());
        let host = TestHost::new(|method,_| match method {
            "media.register" => Ok(json!("stream")),
            "library.import" => Ok(json!({"id":7})),
            _ => Ok(Value::Null),
        });
        for _ in 0..2 {
            p.event(service::Event::Progress { id:"video".into(),path:PathBuf::from("audio.webm") },&host).unwrap();
        }
        assert_eq!(host.count("media.register"),1);
        assert_eq!(host.count("host.play"),1);
        assert_eq!(host.count("library.import"),0);
        assert!(p.saves.contains("video"));
        p.event(service::Event::Download { id:"video".into(),result:Ok(PathBuf::from("audio.webm")) },&host).unwrap();
        assert_eq!(host.count("host.play"),1);
        assert_eq!(host.count("library.import"),1);
        assert!(host.calls.borrow().iter().any(|(name,args)| name == "media.state" && args["state"] == "completed"));
    }
    #[test]
    fn stopping_streaming_preview_preserves_save() {
        let mut p = ready(); p.completed.clear(); p.pending.insert("video".into());
        p.preview_started.insert("video".into()); p.saves.insert("video".into());
        let host = TestHost::new(|_,_| Ok(Value::Null));
        p.action(json!({"action":"play","payload":{"resourceId":"video"}}),&host).unwrap();
        assert_eq!(host.count("host.transport"),1);
        assert!(p.pending.contains("video") && p.saves.contains("video"));
        assert!(!p.preview_started.contains("video"));
    }
    #[test]
    fn stopping_last_streaming_intent_marks_cancelled() {
        let mut p = ready(); p.completed.clear(); p.pending.insert("video".into());
        p.paths.insert("video".into(),PathBuf::from("audio.webm")); p.preview_started.insert("video".into());
        let host = TestHost::new(|_,_| Ok(Value::Null));
        p.action(json!({"action":"play","payload":{"resourceId":"video"}}),&host).unwrap();
        assert!(!p.pending.contains("video") && !p.tokens.contains_key("video"));
        assert!(host.calls.borrow().iter().any(|(name,args)| name == "media.state" && args["state"] == "cancelled"));
    }
    #[test]
    fn cancelling_one_intent_preserves_other() {
        let mut p = ready(); p.pending.insert("video".into()); p.previews.insert("video".into()); p.saves.insert("video".into());
        let host = TestHost::new(|_,_| Ok(Value::Null));
        p.call("action",json!({"action":"play","payload":{"resourceId":"video"}}),&host).unwrap();
        assert!(p.pending.contains("video")); assert!(p.saves.contains("video")); assert!(!p.previews.contains("video"));
    }
    #[test]
    fn cached_download_reused_and_playing_source_adopted() {
        let mut p = ready(); p.playback = json!({"resource":{"pluginId":"player.youtube","resourceId":"video"}});
        let host = TestHost::new(|method,_| if method == "library.import" { Ok(json!({"id":7})) } else { Ok(Value::Null) });
        p.call("action",json!({"action":"save","payload":{"resourceId":"video"}}),&host).unwrap();
        assert_eq!(host.count("library.import"),1); assert_eq!(host.count("host.adopt"),1); assert!(p.pending.is_empty()); assert!(p.saves.is_empty());
    }
    #[test]
    fn simultaneous_preview_and_save_adopts_without_playback_callback() {
        let mut p = ready(); p.previews.insert("video".into()); p.saves.insert("video".into());
        let host = TestHost::new(|method,_| if method == "library.import" { Ok(json!({"id":7})) } else { Ok(Value::Null) });
        p.fulfill("video",&host).unwrap();
        assert_eq!(host.count("host.play"),1); assert_eq!(host.count("host.adopt"),1);
    }
    #[test]
    fn failed_download_clears_both_intents() {
        let mut p = ready(); p.pending.insert("video".into()); p.previews.insert("video".into()); p.saves.insert("video".into());
        let host = TestHost::new(|_,_| Ok(Value::Null));
        assert!(p.event(service::Event::Download{id:"video".into(),result:Err("failure".into())},&host).is_err());
        assert!(p.pending.is_empty() && p.previews.is_empty() && p.saves.is_empty());
    }
    #[test]
    fn stale_search_keeps_its_own_submission() {
        let mut p = Plugin::default(); let host = TestHost::new(|_,_| Ok(Value::Null));
        p.event(service::Event::Search{key:"youtube-search:2".into(),result:Ok(vec![])},&host).unwrap();
        p.event(service::Event::Search{key:"youtube-search:1".into(),result:Err("old failure".into())},&host).unwrap();
        assert!(p.search_rows("youtube-search:2").is_empty());
        assert!(p.search_response("youtube-search:2")["error"].is_null());
        assert_eq!(p.search_response("youtube-search:1")["error"],"old failure");
    }
}
