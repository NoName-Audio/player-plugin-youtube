package dev.verncat.player.plugins.youtube

import android.content.Context
import com.yausername.youtubedl_android.YoutubeDL
import com.yausername.youtubedl_android.YoutubeDLRequest
import java.io.File
import java.io.IOException
import java.util.UUID
import org.json.JSONArray
import org.json.JSONObject

object YoutubeDownloader {
    fun search(context: Context, query: String): JSONArray {
        require(query.isNotBlank() && query.length <= 500) { "Invalid YouTube query" }
        YoutubeDL.init(context)
        val request = YoutubeDLRequest("ytsearch30:$query").apply {
            addOption("--ignore-config")
            addOption("--flat-playlist")
            addOption("--dump-single-json")
            addOption("--skip-download")
        }
        return JSONObject(YoutubeDL.execute(request, redirectErrorStream = false).out).getJSONArray("entries")
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
                val format = if (video) "[ext=mp4][vcodec^=avc1][acodec^=mp4a]" else "[ext=webm][acodec=opus]"
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
