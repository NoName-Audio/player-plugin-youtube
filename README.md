# YouTube plugin for Player
Search YouTube, preview audio, and save tracks from Player on desktop and Android.

In Player versions that support download menus, **Preview** groups formats under **Audio** (WebM/Opus) and **Video** (MP4 with sound), each with a lower-quality **Save data** option. **Save to library** reuses the format selected in **Preview** for that result during the search session, or uses standard WebM/Opus if none was selected. Desktop audio can preview during download; desktop video and Android previews wait for the complete file.

Player builds with native media support embed the title and available artist and album metadata when saving WebM or MP4, without re-encoding. Saving the same YouTube video again in the same library reuses the first import, even if you select a different format or quality. This does not merge older imports.

On Android, install the plugin APK and enable **YouTube** in Player. Open Search, select YouTube, and submit a query.

With a compatible Player build, paste a YouTube playlist URL (including `watch?v=…&list=…`) into the YouTube search tab and press **Search**. The plugin reads up to the first 120 playlist entries. Tick tracks or use **Select available**, then choose **Download selected (audio)** to save them as WebM/Opus. Player checks for existing imports by YouTube video ID and skips them. Files imported outside this plugin or moved out of its import folder are not matched.

On Android, **Share → Player** opens the YouTube search tab for a video or playlist link. If more than one enabled plugin handles the link, Player asks which one to use.

Use the result’s **Preview** menu to choose audio or video and quality, or **Save to library** to save it.

On Android, **Save to library** reuses the latest completed preview if the video ID, format, and quality match and the preview file is still available.

On Android, open **Settings → Plugins → YouTube**. **Sign in via WebView** opens Google’s sign-in page; after returning to YouTube, choose **Use this session**. Google may block sign-in in embedded browsers. Alternatively, select **Import cookies.txt** and choose a Netscape-format cookie file from your signed-in YouTube session. Treat the file like a password. **Remove cookies** clears the session stored for downloads. HTTP 403 can also occur for reasons unrelated to sign-in.

Tagged Android releases use a persistent signing key. If you installed an older CI debug-signed APK, uninstall it once, install the new APK, and approve it in Player. Future releases signed with the same key can update in place.

Release maintainers: configure `ANDROID_PLUGIN_KEYSTORE_BASE64`, `ANDROID_PLUGIN_KEYSTORE_ALIAS`, and `ANDROID_PLUGIN_KEYSTORE_PASSWORD` in the YouTube repository’s Actions secrets. Keep a secure backup of the keystore and reuse it for every release and ABI. The key entry and keystore must use the same password. Never commit the keystore.
