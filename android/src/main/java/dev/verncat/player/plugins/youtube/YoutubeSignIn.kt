package dev.verncat.player.plugins.youtube

import android.app.Activity
import android.app.Dialog
import android.view.ViewGroup.LayoutParams.MATCH_PARENT
import android.view.ViewGroup.LayoutParams.WRAP_CONTENT
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import android.webkit.CookieManager
import android.webkit.WebView
import android.webkit.WebViewClient
import java.util.concurrent.CompletableFuture
import java.util.concurrent.TimeUnit

internal object YoutubeSignIn {
    private const val youtube = "https://www.youtube.com/"
    private const val signIn = "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fwww.youtube.com%2F"

    data class Session(val cookies: ByteArray, val avatarUrl: String?)

    fun open(owner: Activity): Session? {
        val result = CompletableFuture<Session?>()
        val dismiss = CompletableFuture<Dialog>()
        owner.runOnUiThread {
            if (result.isDone || owner.isFinishing || owner.isDestroyed) {
                result.complete(null)
                return@runOnUiThread
            }
            try {
                val web = WebView(owner).apply {
                    settings.javaScriptEnabled = true
                    settings.domStorageEnabled = true
                    settings.allowFileAccess = false
                    settings.allowContentAccess = false
                    webViewClient = object : WebViewClient() {
                        override fun shouldOverrideUrlLoading(view: WebView, request: android.webkit.WebResourceRequest): Boolean {
                            val url = request.url
                            return url.scheme != "https" || url.host !in setOf("accounts.google.com", "www.youtube.com", "m.youtube.com", "youtube.com")
                        }
                    }
                }
                val dialog = Dialog(owner, android.R.style.Theme_Material_NoActionBar)
                val back = Button(owner).apply {
                    text = "Back"
                    setOnClickListener { dialog.dismiss() }
                }
                val save = Button(owner).apply { text = "Use this session" }
                val header = LinearLayout(owner).apply {
                    gravity = android.view.Gravity.CENTER_VERTICAL
                    addView(back, WRAP_CONTENT, WRAP_CONTENT)
                    addView(TextView(owner).apply {
                        text = "YouTube sign-in"
                        textSize = 16f
                        if (android.os.Build.VERSION.SDK_INT >= 28) setAccessibilityHeading(true)
                    }, LinearLayout.LayoutParams(0, WRAP_CONTENT, 1f))
                    addView(save, WRAP_CONTENT, WRAP_CONTENT)
                }
                dialog.setContentView(LinearLayout(owner).apply {
                    orientation = LinearLayout.VERTICAL
                    addView(header, MATCH_PARENT, WRAP_CONTENT)
                    addView(web, LinearLayout.LayoutParams(MATCH_PARENT, 0, 1f))
                })
                dialog.setOnDismissListener {
                    web.stopLoading()
                    (web.parent as? android.view.ViewGroup)?.removeView(web)
                    web.destroy()
                    result.complete(null)
                }
                dismiss.complete(dialog)
                dialog.show()
                dialog.window?.setLayout(MATCH_PARENT, MATCH_PARENT)
                save.setOnClickListener {
                    val pairs = CookieManager.getInstance().getCookie(youtube).orEmpty().split(';')
                        .mapNotNull { entry ->
                            val separator = entry.indexOf('=')
                            if (separator < 1 || entry.any { it < ' ' }) null
                            else entry.substring(0, separator).trim() to entry.substring(separator + 1)
                        }
                    if (pairs.none { it.first == "LOGIN_INFO" && it.second.isNotEmpty() }) {
                        android.widget.Toast.makeText(owner, "Finish signing in to YouTube first", android.widget.Toast.LENGTH_SHORT).show()
                    } else {
                        val jar = buildString {
                            appendLine("# Netscape HTTP Cookie File")
                            for ((name, value) in pairs) {
                                appendLine("www.youtube.com\tFALSE\t/\tTRUE\t0\t$name\t$value")
                            }
                        }
                        val cookies = jar.toByteArray()
                        val page = web.url?.let(android.net.Uri::parse)
                        if (page?.scheme != "https" || page.host !in setOf("www.youtube.com", "m.youtube.com", "youtube.com")) {
                            result.complete(Session(cookies, null))
                            dialog.dismiss()
                        } else {
                            web.evaluateJavascript("document.querySelector('#avatar-btn img, ytm-topbar-menu-button-renderer img')?.currentSrc || null") { value ->
                                val avatar = (runCatching { org.json.JSONTokener(value).nextValue() }.getOrNull() as? String)
                                    ?.takeIf { it.matches(Regex("https://(?:yt3\\.ggpht\\.com|yt3\\.googleusercontent\\.com|lh3\\.googleusercontent\\.com)/[a-zA-Z0-9_./=%-]{1,1000}")) }
                                result.complete(Session(cookies, avatar))
                                dialog.dismiss()
                            }
                        }
                    }
                }
                web.loadUrl(signIn)
            } catch (error: Exception) {
                result.completeExceptionally(error)
            }
        }
        return try {
            result.get(3, TimeUnit.MINUTES)
        } finally {
            result.cancel(false)
            dismiss.thenAccept { dialog -> owner.runOnUiThread { dialog.dismiss() } }
        }
    }
}
