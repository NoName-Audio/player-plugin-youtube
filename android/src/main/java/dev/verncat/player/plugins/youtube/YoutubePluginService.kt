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
import java.util.concurrent.ExecutionException
import java.util.concurrent.FutureTask
import java.util.concurrent.TimeUnit
import java.util.concurrent.TimeoutException
import java.util.concurrent.atomic.AtomicReference
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
        if (method == "canShare") {
            val url = args.optString("url")
            return (playlistSearchUrl(url) != null || videoId(url) != null).toString()
        }
        if (method == "search") {
            val entries = YoutubeDownloader.search(runtime, args.getString("query").trim())
            results.clear()
            for (index in 0 until minOf(entries.length(), SEARCH_RESULT_LIMIT)) {
                val entry = entries.optJSONObject(index) ?: continue
                val id = entry.optString("id").takeIf(videoIdPattern::matches) ?: continue
                results[id] = JSONObject().put("id", id).put("title", entry.optString("title", id))
                    .put("subtitle", entry.optString("uploader", ""))
                    .put("actions", JSONArray().put(JSONObject().put("id", "play").put("label", "Preview").put("downloadOptions", true))
                        .put(JSONObject().put("id", "save").put("label", "Save to library").put("downloadOptions", true)))
            }
            return JSONObject().put("items", JSONArray(results.values)).toString()
        }
        if (method == "renderSearch") return "null"
        if (method != "share" && method != "action") return when (method) {
            "activate", "deactivate", "poll", "playback" -> "null"
            else -> error("Unsupported plugin method: $method")
        }
        val share = method == "share"
        if (share) playlistSearchUrl(args.optString("url"))?.let {
            return JSONObject().put("handled", true).put("searchQuery", it).toString()
        }
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
        val options = if (share) {
            invoke("share.downloadOptions") as? JSONObject ?: return "{\"handled\":true}"
        } else args.getJSONObject("payload")
        val root = File((invoke("storage.staging") as JSONObject).getString("path"))
        val job = File(root, "${if (share) "share" else "result"}-${UUID.randomUUID()}")
        check(root.isAbsolute && job.mkdir()) { "Cannot create YouTube staging directory" }
        var keepForPlayback = false
        try {
            fun progress(percent: Float?, active: Boolean) {
                invoke("ui.downloadProgress", JSONObject().put("title", title)
                    .put("percent", percent?.toDouble() ?: JSONObject.NULL).put("active", active))
            }
            val audio = try {
                progress(null, true)
                downloadWithProgress(runtime, "https://www.youtube.com/watch?v=$id", job,
                    options.optBoolean("video"), options.optBoolean("saveData")) { progress(it, true) }
            } finally {
                progress(null, false)
            }
            val resource = JSONObject().put("pluginId", pluginId()).put("resourceId", id)
                .put("title", title)
                .put("artist", result.optString("subtitle", ""))
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

private fun downloadWithProgress(
    context: Context, url: String, directory: File, video: Boolean, saveData: Boolean,
    report: (Float) -> Unit,
): File {
    // YoutubeDL calls back on its stdout reader. Only the invocation thread calls the host.
    val latest = AtomicReference<Float?>()
    val task = FutureTask {
        YoutubeDownloader.download(context, url, directory, video, saveData, latest::set)
    }
    val worker = Thread(task, "YouTube download").apply { start() }
    var reported: Float? = null
    fun publish() {
        val percent = latest.get() ?: return
        if (percent != reported) {
            report(percent)
            reported = percent
        }
    }
    try {
        while (true) {
            val file = try {
                task.get(200, TimeUnit.MILLISECONDS)
            } catch (_: TimeoutException) {
                publish()
                continue
            } catch (error: ExecutionException) {
                throw error.cause ?: error
            }
            publish()
            return file
        }
    } finally {
        task.cancel(true)
        // Do not delete the job directory while a canceled download is still unwinding.
        worker.join()
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
