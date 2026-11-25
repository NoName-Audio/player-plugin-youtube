# YouTube plugin for Player
Search YouTube, preview audio, and save tracks from Player on desktop and Android.

In Player versions that support download menus, **Preview** groups formats under **Audio** (WebM/Opus) and **Video** (MP4 with sound), each with a lower-quality **Save data** option. **Save to library** reuses the format selected in **Preview** for that result during the search session, or uses standard WebM/Opus if none was selected. Desktop audio can preview during download; desktop video and Android previews wait for the complete file.

Player builds with native media support embed the title and available artist and album metadata when saving WebM or MP4, without re-encoding. Saving the same YouTube video again in the same library reuses the first import, even if you select a different format or quality. This does not merge older imports.

On Android, install the plugin APK and enable **YouTube** in Player. Open Search, select YouTube, and submit a query.

With a compatible Player build, paste a YouTube playlist URL (including `watch?v=…&list=…`) into the YouTube search tab and press **Search**. The plugin reads up to the first 120 playlist entries. Tick tracks or use **Select available**, then choose **Download selected (audio)** to save them as WebM/Opus. Player checks for existing imports by YouTube video ID and skips them. Files imported outside this plugin or moved out of its import folder are not matched.

On Android, **Share → Player** on a playlist link opens the YouTube search results directly if YouTube is the only enabled plugin that handles the link. If more than one plugin matches, Player asks which one to use. Sharing a single-video link without a playlist still opens the download options.

To save audio or video with sound from a link, share it from YouTube or YouTube Music to **Player** and choose the format and data-saving option.
