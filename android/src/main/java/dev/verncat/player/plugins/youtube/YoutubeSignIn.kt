package dev.verncat.player.plugins.youtube

import android.app.Activity
import android.view.ViewGroup
import android.widget.FrameLayout
import android.webkit.CookieManager
import android.webkit.WebView
import android.webkit.WebViewClient
import android.webkit.WebResourceRequest
import java.util.concurrent.FutureTask
import java.util.concurrent.TimeUnit

internal class YoutubeSignIn {
    private var web: WebView? = null
    private val youtube = "https://www.youtube.com/"

    private fun <T> ui(owner: Activity, block: () -> T): T {
        val task = FutureTask(block)
        owner.runOnUiThread(task)
        return try { task.get(5, TimeUnit.SECONDS) } finally { task.cancel(false) }
    }

    fun position(owner: Activity, x: Int, y: Int, width: Int, height: Int) = ui(owner) {
        val root = owner.findViewById<FrameLayout>(android.R.id.content)
        val browser = web ?: WebView(owner).apply {
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            settings.allowFileAccess = false
            settings.allowContentAccess = false
            webViewClient = object : WebViewClient() {
                override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                    val url = request.url
                    return url.scheme != "https" || url.host !in setOf("accounts.google.com", "www.youtube.com", "m.youtube.com", "youtube.com", "consent.youtube.com", "consent.google.com")
                }
            }
            root.addView(this)
            web = this
            loadUrl(youtube)
        }
        browser.layoutParams = FrameLayout.LayoutParams(width, height).apply {
            leftMargin = x
            topMargin = y
        }
        browser.visibility = android.view.View.VISIBLE
    }

    fun hide(owner: Activity) = ui(owner) { web?.visibility = android.view.View.GONE; Unit }
    fun close(owner: Activity) = ui(owner) {
        web?.let {
            (it.parent as? ViewGroup)?.removeView(it)
            it.destroy()
        }
        web = null
    }

    data class Session(val cookies: ByteArray, val avatarUrl: String?)

    fun session(owner: Activity): Session {
        val result = java.util.concurrent.CompletableFuture<String?>()
        val cookies = ui(owner) {
            val browser = checkNotNull(web) { "YouTube page is closed" }
            val pairs = CookieManager.getInstance().getCookie(youtube).orEmpty().split(';').mapNotNull { entry ->
                val separator = entry.indexOf('=')
                if (separator < 1 || entry.any { it < ' ' }) null
                else entry.substring(0, separator).trim() to entry.substring(separator + 1)
            }
            check(pairs.any { it.first == "LOGIN_INFO" && it.second.isNotEmpty() }) { "Finish signing in to YouTube first" }
            val url = browser.url?.let(android.net.Uri::parse)
            if (url?.scheme == "https" && url.host in setOf("www.youtube.com", "m.youtube.com", "youtube.com")) {
                browser.evaluateJavascript("document.querySelector('#avatar-btn img, ytm-topbar-menu-button-renderer img')?.currentSrc || null") { value ->
                    result.complete((runCatching { org.json.JSONTokener(value).nextValue() }.getOrNull() as? String)
                        ?.takeIf { it.matches(Regex("https://(?:yt3\\.ggpht\\.com|yt3\\.googleusercontent\\.com|lh3\\.googleusercontent\\.com)/[a-zA-Z0-9_./=%-]{1,1000}")) })
                }
            } else result.complete(null)
            buildString {
                appendLine("# Netscape HTTP Cookie File")
                for ((name, value) in pairs) appendLine("www.youtube.com\tFALSE\t/\tTRUE\t0\t$name\t$value")
            }.toByteArray()
        }
        return Session(cookies, result.completeOnTimeout(null, 2, TimeUnit.SECONDS).get())
    }
}
