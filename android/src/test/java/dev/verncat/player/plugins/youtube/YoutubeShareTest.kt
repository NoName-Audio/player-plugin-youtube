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

    @Test fun canonicalizesPlaylistSearchLinks() {
        val list = "PLabc-123_xyz"
        val canonical = "https://www.youtube.com/playlist?list=$list"
        for (url in listOf(canonical, "http://M.YOUTUBE.COM/playlist?si=x&list=$list#fragment",
            "https://music.youtube.com/watch?v=$id&list=$list&index=7",
            "https://youtube.com/watch?list=$list")) {
            assertEquals(url, canonical, playlistSearchUrl(url))
        }
    }

    @Test fun rejectsMalformedAndUnrelatedPlaylistSearchLinks() {
        for (url in listOf("https://youtube.com.evil.test/playlist?list=PLabc",
            "https://user@youtube.com/playlist?list=PLabc", "https://youtube.com:443/playlist?list=PLabc",
            "https://youtu.be/$id?list=PLabc", "https://youtube.com/playlist/extra?list=PLabc",
            "https://youtube.com/watch?v=$id", "https://youtube.com/playlist?list=",
            "https://youtube.com/playlist?list=PLabc&list", "https://youtube.com/playlist?list=PL%61bc",
            "https://youtube.com/playlist?list=PLabc&list=PLother", "ftp://youtube.com/playlist?list=PLabc",
            "https://youtube.com/playlist?list=${"a".repeat(129)}")) {
            assertNull(url, playlistSearchUrl(url))
        }
    }
}
