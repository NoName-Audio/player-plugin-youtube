# YouTube plugin for Player
Search YouTube, preview audio, and save tracks from Player on desktop and Android.

In Player versions that support download menus, **Preview** groups formats under **Audio** (WebM/Opus) and **Video** (MP4 with sound), each with a lower-quality **Save data** option. **Save to library** reuses the format selected in **Preview** for that result during the search session, or uses standard WebM/Opus if none was selected. Desktop audio can preview during download; desktop video and Android previews wait for the complete file.

Player builds with native media support embed the title and available artist and album metadata when saving WebM or MP4, without re-encoding. Saving the same YouTube video again in the same library reuses the first import, even if you select a different format or quality. This does not merge older imports.

On Android, install the plugin APK and enable **YouTube** in Player. Open Search, select YouTube, and submit a query.

With a compatible Player build, paste a YouTube playlist URL (including `watch?v=…&list=…`) into the YouTube search tab and press **Search**. The plugin reads up to the first 120 playlist entries. Tick tracks or use **Select available**, then choose **Download selected (audio)** to save them as WebM/Opus. Player checks for existing imports by YouTube video ID and skips them. Files imported outside this plugin or moved out of its import folder are not matched.

On Android, **Share → Player** opens the YouTube search tab for a video or playlist link. If more than one enabled plugin handles the link, Player asks which one to use.

Use the result’s **Preview** menu to choose audio or video and quality, or **Save to library** to save it.
