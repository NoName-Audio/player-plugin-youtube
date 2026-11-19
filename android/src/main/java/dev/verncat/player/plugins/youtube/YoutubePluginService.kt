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
import org.json.JSONObject

/** API v2 reflective entry point. Host callbacks are valid only during call(). */
class YoutubePluginService(context: Context) {
    private val runtime = YoutubeContext(context.applicationContext)

    fun apiVersion() = 2
    fun pluginId() = "player.youtube"
    fun close() = Unit // No retained callbacks or background jobs.

    fun call(method: String, argsJson: String, host: (String, String) -> String): String {
        if (method == "canShare") return (videoId(JSONObject(argsJson).optString("url")) != null).toString()
        if (method != "share") return when (method) {
            "activate", "deactivate", "poll", "playback" -> "null"
            else -> error("Unsupported plugin method: $method")
        }
        val id = videoId(JSONObject(argsJson).optString("url")) ?: return "{\"handled\":false}"
        fun invoke(name: String, args: JSONObject = JSONObject()): Any {
            val response = JSONObject(host(name, args.toString()))
            check(response.getBoolean("ok")) { response.optString("error", "$name failed") }
            return response.get("value")
        }
        val options = invoke("share.downloadOptions") as? JSONObject ?: return "{\"handled\":true}"
        val root = File((invoke("storage.staging") as JSONObject).getString("path"))
        val job = File(root, "share-${UUID.randomUUID()}")
        check(root.isAbsolute && job.mkdir()) { "Cannot create YouTube staging directory" }
        try {
            val audio = YoutubeDownloader.download(runtime, "https://www.youtube.com/watch?v=$id", job,
                options.optBoolean("video"), options.optBoolean("saveData"))
            val resource = JSONObject().put("pluginId", pluginId()).put("resourceId", id)
                .put("title", audio.nameWithoutExtension)
            val token = invoke("media.register", JSONObject().put("path", audio.absolutePath)
                .put("state", "completed").put("resource", resource)) as String
            check(invoke("library.import", JSONObject().put("sourceToken", token)) is JSONObject) {
                "Player did not confirm library import"
            }
            return "{\"handled\":true}"
        } finally {
            job.deleteRecursively()
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
