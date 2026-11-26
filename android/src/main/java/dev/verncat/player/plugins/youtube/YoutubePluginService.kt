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
    private var activity: java.lang.ref.WeakReference<Activity>? = null

    constructor(owner: Activity) : this(owner.applicationContext) {
        activity = java.lang.ref.WeakReference(owner)
    }
    private val signIn = YoutubeSignIn()

    fun apiVersion() = 2
    fun pluginId() = "player.youtube"
    fun close() { activity?.get()?.let(signIn::close) }

    private val results = LinkedHashMap<String, JSONObject>()
    private data class DownloadKey(val id: String, val video: Boolean, val saveData: Boolean)
    private var preview: Pair<DownloadKey, File>? = null

    fun call(method: String, argsJson: String, host: (String, String) -> String): String {
        val args = JSONObject(argsJson)
        fun invoke(name: String, args: JSONObject = JSONObject()): Any {
            val response = JSONObject(host(name, args.toString()))
            check(response.getBoolean("ok")) { response.optString("error", "$name failed") }
            return response.get("value")
        }
        val cookieKey = JSONObject().put("key", "youtube-cookies")
        val avatarKey = JSONObject().put("key", "youtube-avatar")
        if (method == "render") {
            if (args.optString("viewId") != "authorization") return "null"
            val configured = invoke("secrets.has", cookieKey) == true
            val account = JSONObject().put("type", "account")
                .put("text", if (configured) "YouTube session saved" else "Not signed in to YouTube")
            if (configured && invoke("secrets.has", avatarKey) == true) {
                account.put("avatarUrl", invoke("secrets.get", avatarKey))
            }
            val accountActions = JSONArray().put(account)
                .put(JSONObject().put("type", "button").put("action", "web-sign-in").put("label", "Sign in via WebView"))
            if (configured) accountActions.put(JSONObject().put("type", "button").put("action", "delete-cookies").put("label", "Remove cookies"))
            val importActions = JSONArray()
                .put(JSONObject().put("type", "text").put("text", "Use a Netscape-format cookies.txt from your YouTube session. Treat the file like a password."))
                .put(JSONObject().put("type", "input").put("key", "cookies").put("inputType", "text-file").put("action", "set-cookies").put("label", "Import cookies.txt"))
            return JSONObject().put("type", "group").put("layout", "cards").put("children", JSONArray()
                .put(JSONObject().put("type", "group").put("layout", "card").put("title", "YouTube account").put("children", accountActions))
                .put(JSONObject().put("type", "group").put("layout", "card").put("title", "Downloader").put("children", JSONArray()
                    .put(JSONObject().put("type", "text").put("text", runCatching { "yt-dlp ${YoutubeDownloader.version(runtime)}" }
                        .getOrElse { "Could not read yt-dlp version: ${it.message}" }))
                    .apply { YoutubeDownloader.updateError?.let { put(JSONObject().put("type", "text").put("tone", "error").put("text", it)) } }
                    .put(JSONObject().put("type", "button").put("action", "update-downloader").put("label", "Update yt-dlp"))))
                .put(JSONObject().put("type", "group").put("layout", "card").put("title", "Manual import").put("children", importActions))).toString()
        }
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
        if (method == "share") {
            val url = args.optString("url")
            val query = playlistSearchUrl(url) ?: videoId(url)?.let { "https://www.youtube.com/watch?v=$it" }
                ?: return "{\"handled\":false}"
            return JSONObject().put("handled", true).put("searchQuery", query).toString()
        }
        val action = args.getString("action")
        if (action == "update-downloader") {
            YoutubeDownloader.update(runtime)
            return "null"
        }
        if (action == "web-sign-in") {
            invoke("host.navigate", JSONObject().put("pageId", "sign-in"))
            return "null"
        }
        if (action in setOf("sign-in-position", "sign-in-close", "sign-in-save", "sign-in-hide")) {
            val owner = activity?.get() ?: error("WebView sign-in requires an updated Player")
            if (action == "sign-in-hide") {
                signIn.hide(owner)
                return "null"
            }
            if (action == "sign-in-close") {
                signIn.close(owner)
                return "null"
            }
            if (action == "sign-in-position") {
                val rect = args.getJSONObject("payload")
                signIn.position(owner, rect.getInt("x"), rect.getInt("y"), rect.getInt("width"), rect.getInt("height"))
                return "null"
            }
            val session = signIn.session(owner)
            invoke("secrets.delete", avatarKey)
            invoke("secrets.set", cookieKey.put("value", android.util.Base64.encodeToString(session.cookies, android.util.Base64.NO_WRAP)))
            session.avatarUrl?.let { invoke("secrets.set", avatarKey.put("value", it)) }
            return "null"
        }
        if (action == "set-cookies") {
            val text = args.getJSONObject("payload").getString("text").removePrefix("\uFEFF").replace("\r\n", "\n")
            val bytes = text.toByteArray(Charsets.UTF_8)
            require(bytes.size <= 64 * 1024 && text.lineSequence().firstOrNull() in listOf("# Netscape HTTP Cookie File", "# HTTP Cookie File")) {
                "Expected a Netscape cookies.txt file no larger than 64 KiB"
            }
            invoke("secrets.delete", avatarKey)
            invoke("secrets.set", cookieKey.put("value", android.util.Base64.encodeToString(bytes, android.util.Base64.NO_WRAP)))
            return "null"
        }
        if (action == "delete-cookies") {
            invoke("secrets.delete", avatarKey)
            invoke("secrets.delete", cookieKey)
            return "null"
        }
        require(action == "save" || action == "play") { "Unknown YouTube action" }
        val options = args.getJSONObject("payload")
        val id = options.getString("resourceId")
        val result = requireNotNull(results[id]) { "Search again to restore this result" }
        val title = result.optString("title", id)
        val key = DownloadKey(id, options.optBoolean("video"), options.optBoolean("saveData"))
        val resource = JSONObject().put("pluginId", pluginId()).put("resourceId", id)
            .put("title", title)
            .put("artist", result.optString("subtitle", ""))
        fun register(file: File): String = invoke("media.register", JSONObject().put("path", file.absolutePath)
            .put("state", "completed").put("resource", resource)) as String
        fun save(token: String) {
            check(invoke("library.import", JSONObject().put("sourceToken", token)) is JSONObject) {
                "Player did not confirm library import"
            }
        }
        val cached = preview?.takeIf { it.first == key }?.second
        if (action == "save" && cached != null && cached.isFile && cached.length() > 0) {
            save(register(cached))
            return "null"
        }
        val root = File((invoke("storage.staging") as JSONObject).getString("path"))
        val job = File(root, "result-${UUID.randomUUID()}")
        check(root.isAbsolute && job.mkdir()) { "Cannot create YouTube staging directory" }
        var keepForPlayback = false
        try {
            fun progress(percent: Float?, active: Boolean) {
                invoke("ui.downloadProgress", JSONObject().put("title", title)
                    .put("percent", percent?.toDouble() ?: JSONObject.NULL).put("active", active))
            }
            val cookies = if (invoke("secrets.has", cookieKey) == true) {
                android.util.Base64.decode(invoke("secrets.get", cookieKey) as String, android.util.Base64.DEFAULT)
            } else null
            val audio = try {
                progress(null, true)
                downloadWithProgress(runtime, "https://www.youtube.com/watch?v=$id", job,
                    key.video, key.saveData, cookies) { progress(it, true) }
            } finally {
                progress(null, false)
            }
            val token = register(audio)
            if (action == "play") {
                invoke("host.play", JSONObject().put("sourceToken", token).put("resource", resource))
                keepForPlayback = true
                preview = key to audio
                return "null"
            }
            save(token)
            return "null"
        } finally {
            if (!keepForPlayback) job.deleteRecursively()
        }
    }
}

private fun downloadWithProgress(
    context: Context, url: String, directory: File, video: Boolean, saveData: Boolean, cookies: ByteArray?,
    report: (Float) -> Unit,
): File {
    // YoutubeDL calls back on its stdout reader. Only the invocation thread calls the host.
    val latest = AtomicReference<Float?>()
    val task = FutureTask {
        YoutubeDownloader.download(context, url, directory, video, saveData, cookies, latest::set)
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
