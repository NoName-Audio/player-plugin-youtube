package dev.verncat.player.plugins.youtube

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class YoutubeShareTest {
    private val id = "A-b_c012345"

    @Test fun acceptsVideoLinksWithoutPassingSharedOptionsToDownloader() {
        for (url in listOf("https://youtu.be/$id?t=12", "https://www.youtube.com/watch?list=abc&v=$id",
            "https://music.youtube.com/watch?v=$id", "https://m.youtube.com/shorts/$id", "https://youtube.com/live/$id")) {
            assertEquals(url, id, videoId(url))
        }
    }

    @Test fun declinesUnrelatedSharesAndAmbiguousVideos() {
        for (url in listOf("", "content://provider/$id", "https://youtube.com.evil.test/watch?v=$id",
            "https://youtube.com@evil.test/watch?v=$id", "https://evil.test@youtube.com/watch?v=$id",
            "https://youtube.com/playlist?list=$id", "https://youtube.com/watch?v=$id&v=$id",
            "https://youtu.be/not-an-id", "https://youtu.be/$id/extra", "https://youtu.be/%")) {
            assertNull(url, videoId(url))
        }
    }
}
