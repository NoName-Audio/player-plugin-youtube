package dev.verncat.player.plugins.youtube

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.content.pm.ApplicationInfo
import android.content.res.Resources
import java.net.URI
import java.net.URISyntaxException
import android.os.Bundle
import java.io.File
import java.util.UUID
import org.json.JSONArray
import org.json.JSONObject

/** API v2 reflective entry point. Host callbacks are valid only during call(). */
class YoutubePluginService(context: Context) {
    private val runtime = YoutubeContext(context.applicationContext)

    fun apiVersion() = 2
    fun pluginId() = "player.youtube"
    fun close() = Unit // No retained callbacks or background jobs.

    private val results = LinkedHashMap<String, JSONObject>()

    fun call(method: String, argsJson: String, host: (String, String) -> String): String {
        val args = JSONObject(argsJson)
        if (method == "canShare") return (videoId(args.optString("url")) != null).toString()
        if (method == "search") {
            val entries = YoutubeDownloader.search(runtime, args.getString("query").trim())
            results.clear()
            for (index in 0 until entries.length()) {
                val entry = entries.optJSONObject(index) ?: continue
                val id = entry.optString("id").takeIf(videoIdPattern::matches) ?: continue
                results[id] = JSONObject().put("id", id).put("title", entry.optString("title", id))
                    .put("subtitle", entry.optString("uploader", ""))
                    .put("actions", JSONArray().put(JSONObject().put("id", "play").put("label", "Preview"))
                        .put(JSONObject().put("id", "save").put("label", "Save to library")))
            }
            return JSONObject().put("items", JSONArray(results.values)).toString()
        }
        if (method == "renderSearch") return "null"
        if (method != "share" && method != "action") return when (method) {
            "activate", "deactivate", "poll", "playback" -> "null"
            else -> error("Unsupported plugin method: $method")
        }
        val share = method == "share"
        val action = if (share) "save" else args.getString("action")
        require(action == "save" || action == "play") { "Unknown YouTube action" }
        val id = if (share) videoId(args.optString("url")) ?: return "{\"handled\":false}"
            else args.getJSONObject("payload").getString("resourceId").also {
                require(results.containsKey(it)) { "Search again to restore this result" }
            }
        fun invoke(name: String, args: JSONObject = JSONObject()): Any {
            val response = JSONObject(host(name, args.toString()))
            check(response.getBoolean("ok")) { response.optString("error", "$name failed") }
            return response.get("value")
        }
        val optionReply = JSONObject(host("share.downloadOptions", "{}"))
        val options = when {
            optionReply.getBoolean("ok") -> optionReply.optJSONObject("value")
            !share && optionReply.optString("error") == "METHOD_NOT_FOUND" -> JSONObject()
            else -> error(optionReply.optString("error", "Download options unavailable"))
        } ?: return if (share) "{\"handled\":true}" else "null"
        val root = File((invoke("storage.staging") as JSONObject).getString("path"))
        val job = File(root, "${if (share) "share" else "result"}-${UUID.randomUUID()}")
        check(root.isAbsolute && job.mkdir()) { "Cannot create YouTube staging directory" }
        var keepForPlayback = false
        try {
            val audio = YoutubeDownloader.download(runtime, "https://www.youtube.com/watch?v=$id", job,
                options.optBoolean("video"), options.optBoolean("saveData"))
            val resource = JSONObject().put("pluginId", pluginId()).put("resourceId", id)
                .put("title", results[id]?.optString("title") ?: audio.nameWithoutExtension)
                .put("artist", results[id]?.optString("subtitle") ?: "")
            val token = invoke("media.register", JSONObject().put("path", audio.absolutePath)
                .put("state", "completed").put("resource", resource)) as String
            if (action == "play") {
                invoke("host.play", JSONObject().put("sourceToken", token).put("resource", resource))
                keepForPlayback = true
                return "null"
            }
            check(invoke("library.import", JSONObject().put("sourceToken", token)) is JSONObject) {
                "Player did not confirm library import"
            }
            return if (share) "{\"handled\":true}" else "null"
        } finally {
            if (!keepForPlayback) job.deleteRecursively()
        }
    }
}

private val videoIdPattern = Regex("[A-Za-z0-9_-]{11}")
internal fun videoId(url: String): String? {
    val uri = try { URI(url) } catch (_: URISyntaxException) { return null }
    if (!uri.scheme.equals("https", true) && !uri.scheme.equals("http", true)) return null
    val parts = uri.rawPath?.trim('/')?.split('/') ?: return null
    val id = when (uri.rawAuthority?.lowercase()) {
        "youtu.be" -> parts.singleOrNull()
        "youtube.com", "www.youtube.com", "m.youtube.com", "music.youtube.com" -> when {
            parts == listOf("watch") -> uri.rawQuery?.split('&')?.singleOrNull { it.startsWith("v=") }?.substring(2)
            parts.size == 2 && parts[0] in setOf("shorts", "live", "embed") -> parts[1]
            else -> null
        }
        else -> null
    }
    return id?.takeIf(videoIdPattern::matches)
}

/** Execute installed APK binaries and read its resources, but keep all mutable state under Player's UID. */
private class YoutubeContext(host: Context) : ContextWrapper(host) {
    private val apk = host.createPackageContext("dev.verncat.player.plugins.youtube", 0)
    private val work = host.getDir("player_youtube", Context.MODE_PRIVATE)
    override fun getApplicationContext(): Context = this
    override fun getApplicationInfo(): ApplicationInfo = apk.applicationInfo
    override fun getResources(): Resources = apk.resources
    override fun getAssets() = apk.assets
    override fun getNoBackupFilesDir(): File = work
    override fun getFilesDir(): File = work
}

/** Discovery marker for Player; not a second Android share target. */
class DiscoveryActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) { super.onCreate(savedInstanceState); finish() }
}
