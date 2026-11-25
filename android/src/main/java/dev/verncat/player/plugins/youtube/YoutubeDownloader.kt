package dev.verncat.player.plugins.youtube

import android.content.Context
import com.yausername.youtubedl_android.YoutubeDL
import com.yausername.youtubedl_android.YoutubeDLRequest
import java.io.File
import java.io.IOException
import java.net.URI
import java.net.URISyntaxException
import java.util.UUID
import org.json.JSONArray
import org.json.JSONObject

internal const val SEARCH_RESULT_LIMIT = 120
private val playlistIdPattern = Regex("[A-Za-z0-9_-]{1,128}")

internal fun playlistSearchUrl(query: String): String? {
    val uri = try { URI(query) } catch (_: URISyntaxException) { return null }
    if (!uri.scheme.equals("https", true) && !uri.scheme.equals("http", true)) return null
    when (uri.rawAuthority?.lowercase()) {
        "youtube.com", "www.youtube.com", "m.youtube.com", "music.youtube.com" -> Unit
        else -> return null
    }
    if (uri.rawPath != "/playlist" && uri.rawPath != "/watch") return null
    val params = uri.rawQuery?.split('&') ?: return null
    val list = params.singleOrNull { it.substringBefore('=') == "list" }
        ?.substringAfter('=', "")?.takeIf(playlistIdPattern::matches) ?: return null
    return "https://www.youtube.com/playlist?list=$list"
}

object YoutubeDownloader {
    fun search(context: Context, query: String): JSONArray {
        require(query.isNotBlank() && query.length <= 500) { "Invalid YouTube query" }
        val playlistUrl = playlistSearchUrl(query)
        val videoUrl = if (playlistUrl == null) videoId(query)?.let { "https://www.youtube.com/watch?v=$it" } else null
        YoutubeDL.init(context)
        val request = YoutubeDLRequest(playlistUrl ?: videoUrl ?: "ytsearch30:$query").apply {
            addOption("--ignore-config")
            addOption("--flat-playlist")
            addOption("--dump-single-json")
            addOption("--skip-download")
            if (playlistUrl != null) {
                addOption("--yes-playlist")
                addOption("--playlist-items", "1:$SEARCH_RESULT_LIMIT")
            }
            if (videoUrl != null) addOption("--no-playlist")
        }
        val response = JSONObject(YoutubeDL.execute(request, redirectErrorStream = false).out)
        return if (videoUrl != null) JSONArray().put(response) else response.getJSONArray("entries")
    }

    /** Blocking; returns a completed media file owned by the caller. */
    fun download(context: Context, url: String, directory: File, video: Boolean, saveData: Boolean): File {
        require(url.startsWith("https://", ignoreCase = true) || url.startsWith("http://", ignoreCase = true)) {
            "Expected an HTTP or HTTPS video URL"
        }
        YoutubeDL.init(context)
        if (!directory.isDirectory && !directory.mkdirs() && !directory.isDirectory) {
            throw IOException("Cannot create download directory: $directory")
        }
        val id = UUID.randomUUID().toString()
        val stage = File(directory, ".download-$id")
        if (!stage.mkdir()) throw IOException("Cannot create download staging directory")
        try {
            val request = YoutubeDLRequest(url).apply {
                addOption("--ignore-config")
                addOption("--no-playlist")
                addOption("--playlist-items", "1")
                addOption("--no-progress")
                val format = if (video) "[ext=mp4][vcodec^=avc1][acodec^=mp4a]" else "[ext=webm][acodec=opus][vcodec=none]"
                val best = if (video) "best" else "bestaudio"
                val worst = if (video) "worst" else "worstaudio"
                addOption("--format", (if (saveData) worst else best) + format)
                addOption("--fixup", "never")
                addOption("--paths", stage.absolutePath)
                addOption("--restrict-filenames")
                addOption("--output", "%(title).100B [%(id)s].%(ext)s")
            }
            YoutubeDL.execute(request, redirectErrorStream = false)
            val extension = if (video) "mp4" else "webm"
            val audio = stage.listFiles()?.singleOrNull { it.isFile && it.extension == extension }
                ?: throw IOException("yt-dlp did not produce one $extension file")
            val result = File(directory, audio.name)
            if (audio.length() == 0L || !audio.renameTo(result)) {
                throw IOException("Could not publish downloaded audio")
            }
            return result
        } finally {
            stage.deleteRecursively()
        }
    }
}
