#[cfg(unix)]
mod test {
    use std::{
        ffi::OsString,
        fs,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
    };

    use assert_cmd::Command;
    use tempfile::TempDir;

    struct Sandbox {
        _tempdir: TempDir,
        home: PathBuf,
        config: PathBuf,
        data: PathBuf,
        cache: PathBuf,
        pictures: PathBuf,
        bin: PathBuf,
        state: PathBuf,
    }

    impl Sandbox {
        fn new() -> Self {
            let tempdir = tempfile::tempdir().unwrap();
            let root = tempdir.path().to_path_buf();
            let home = root.join("home");
            let config = root.join("config");
            let data = root.join("data");
            let cache = root.join("cache");
            let pictures = home.join("Pictures");
            let bin = root.join("bin");
            let state = root.join("state");

            fs::create_dir_all(home.join(".config")).unwrap();
            fs::create_dir_all(&config).unwrap();
            fs::create_dir_all(&data).unwrap();
            fs::create_dir_all(&cache).unwrap();
            fs::create_dir_all(&pictures).unwrap();
            fs::create_dir_all(&bin).unwrap();
            fs::create_dir_all(&state).unwrap();
            fs::write(
                config.join("user-dirs.dirs"),
                "XDG_PICTURES_DIR=\"$HOME/Pictures\"\n",
            )
            .unwrap();

            write_script(
                &bin.join("swaymsg"),
                "#!/bin/sh\nprintf '%s\n' '[{\"name\":\"DP-1\",\"rect\":{\"width\":1920}},{\"name\":\"HDMI-A-1\",\"rect\":{\"width\":1280}}]'\n",
            );
            write_script(
                &bin.join("xrandr"),
                "#!/bin/sh\nprintf '%s\n' 'DP-1 connected primary 1920x1080+0+0'\nprintf '%s\n' 'HDMI-A-1 connected 1280x1024+1920+0'\n",
            );
            write_script(
                &bin.join("convert"),
                "#!/bin/sh\n: > \"$TEST_STATE/convert.args\"\nlast=''\nfor arg in \"$@\"; do\n  printf '%s\n' \"$arg\" >> \"$TEST_STATE/convert.args\"\n  last=\"$arg\"\ndone\nmkdir -p \"$(dirname \"$last\")\"\nprintf 'rendered\n' > \"$last\"\n",
            );
            write_script(
                &bin.join("swaylock"),
                "#!/bin/sh\n: > \"$TEST_STATE/swaylock.args\"\nfor arg in \"$@\"; do\n  printf '%s\n' \"$arg\" >> \"$TEST_STATE/swaylock.args\"\ndone\n",
            );
            write_script(
                &bin.join("i3lock"),
                "#!/bin/sh\n: > \"$TEST_STATE/i3lock.args\"\nfor arg in \"$@\"; do\n  printf '%s\n' \"$arg\" >> \"$TEST_STATE/i3lock.args\"\ndone\n",
            );

            Self {
                _tempdir: tempdir,
                home,
                config,
                data,
                cache,
                pictures,
                bin,
                state,
            }
        }

        fn command(&self) -> Command {
            let mut command = Command::cargo_bin("xkcd_lock").unwrap();
            command
                .env("HOME", &self.home)
                .env("XDG_CONFIG_HOME", &self.config)
                .env("XDG_DATA_HOME", &self.data)
                .env("XDG_CACHE_HOME", &self.cache)
                .env("XDG_PICTURES_DIR", self.pictures.as_os_str())
                .env("XKCD_LOCK_TRACE_LOG_PATH", self.trace_log().as_os_str())
                .env("TEST_STATE", &self.state)
                .env("PATH", test_path(&self.bin));
            command
        }

        fn xkcd_cache(&self) -> PathBuf {
            self.pictures.join("xkcd")
        }

        fn trace_log(&self) -> PathBuf {
            self.state.join("xkcd_lock.trace.log")
        }
    }

    #[test]
    fn cached_random_comic_flows_through_renderer_and_swaylock() {
        let sandbox = Sandbox::new();
        let cache = sandbox.xkcd_cache();
        fs::create_dir_all(cache.join("latest")).unwrap();
        fs::write(cache.join("latest").join("keep"), 1u32.to_le_bytes()).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("0001 - Cached Comic.png"), "cached").unwrap();
        fs::create_dir_all(cache.join("metadata")).unwrap();
        fs::write(
            cache.join("metadata").join("0001.json"),
            "{\"img\":\"https://imgs.xkcd.com/comics/cached.png\",\"title\":\"Cached Comic\",\"alt\":\"Cached alt text\",\"num\":1}",
        )
        .unwrap();

        sandbox
            .command()
            .env("XDG_SESSION_TYPE", "wayland")
            .assert()
            .success();

        let rendered = cache.join("with_text").join("0001 - Cached Comic.png");
        assert!(rendered.exists());

        let swaylock = fs::read_to_string(sandbox.state.join("swaylock.args")).unwrap();
        assert!(swaylock.contains(&format!("DP-1:{}", rendered.display())));
        assert!(swaylock.contains(&format!("HDMI-A-1:{}", rendered.display())));

        let convert = fs::read_to_string(sandbox.state.join("convert.args")).unwrap();
        assert!(convert.contains("Cached Comic"));
        assert!(convert.contains("Cached alt text"));
    }

    #[test]
    fn explicit_i3_uses_image_override_without_rendering() {
        let sandbox = Sandbox::new();
        let image = sandbox.pictures.join("custom.png");
        fs::write(&image, "local image").unwrap();

        sandbox
            .command()
            .args(["--image", image.to_str().unwrap(), "i3"])
            .assert()
            .success();

        assert!(!sandbox.state.join("convert.args").exists());

        let i3lock = fs::read_to_string(sandbox.state.join("i3lock.args")).unwrap();
        assert!(i3lock.contains("-i\n"));
        assert!(i3lock.contains(&format!("{}\n", image.display())));
        assert!(!i3lock.contains("DP-1:"));
        assert!(!i3lock.contains("HDMI-A-1:"));
    }

    #[test]
    fn trace_log_is_written_unconditionally() {
        let sandbox = Sandbox::new();
        let image = sandbox.pictures.join("custom.png");
        fs::write(&image, "local image").unwrap();

        sandbox
            .command()
            .args(["--image", image.to_str().unwrap(), "i3"])
            .assert()
            .success();

        let trace_log = fs::read_to_string(sandbox.trace_log()).unwrap();
        assert!(trace_log.contains("Parsed CLI options"));
        assert!(trace_log.contains("Using image override"));
        assert!(trace_log.contains("Starting lockscreen"));
    }

    #[test]
    fn quiet_still_writes_trace_log() {
        let sandbox = Sandbox::new();
        let image = sandbox.pictures.join("custom.png");
        fs::write(&image, "local image").unwrap();

        sandbox
            .command()
            .args(["-q", "--image", image.to_str().unwrap(), "i3"])
            .assert()
            .success();

        let trace_log = fs::read_to_string(sandbox.trace_log()).unwrap();
        assert!(trace_log.contains("Configured trace log file"));
        assert!(trace_log.contains("Parsed CLI options"));
        assert!(trace_log.contains("Resolved lock backend"));
    }

    #[test]
    fn i3_does_not_need_output_detection() {
        let sandbox = Sandbox::new();
        write_script(
            &sandbox.bin.join("swaymsg"),
            "#!/bin/sh\nprintf '%s\n' 'swaymsg should not be called' >&2\nexit 1\n",
        );
        write_script(
            &sandbox.bin.join("xrandr"),
            "#!/bin/sh\nprintf '%s\n' 'xrandr should not be called' >&2\nexit 1\n",
        );
        let image = sandbox.pictures.join("custom.png");
        fs::write(&image, "local image").unwrap();

        sandbox
            .command()
            .args(["--image", image.to_str().unwrap(), "i3"])
            .assert()
            .success();

        let i3lock = fs::read_to_string(sandbox.state.join("i3lock.args")).unwrap();
        assert!(i3lock.contains("-i\n"));
        assert!(i3lock.contains(&format!("{}\n", image.display())));
        assert!(!i3lock.contains("DP-1:"));
        assert!(!i3lock.contains("HDMI-A-1:"));
    }

    #[test]
    fn failed_render_does_not_publish_partial_background() {
        let sandbox = Sandbox::new();
        write_script(
            &sandbox.bin.join("convert"),
            "#!/bin/sh\nlast=''\nfor arg in \"$@\"; do\n  last=\"$arg\"\ndone\nmkdir -p \"$(dirname \"$last\")\"\nprintf 'partial\n' > \"$last\"\nexit 1\n",
        );
        let cache = sandbox.xkcd_cache();
        fs::create_dir_all(cache.join("latest")).unwrap();
        fs::write(cache.join("latest").join("keep"), 1u32.to_le_bytes()).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("0001 - Cached Comic.png"), "cached").unwrap();
        fs::create_dir_all(cache.join("metadata")).unwrap();
        fs::write(
            cache.join("metadata").join("0001.json"),
            "{\"img\":\"https://imgs.xkcd.com/comics/cached.png\",\"title\":\"Cached Comic\",\"alt\":\"Cached alt text\",\"num\":1}",
        )
        .unwrap();

        sandbox
            .command()
            .env("XDG_SESSION_TYPE", "wayland")
            .assert()
            .failure();

        let rendered = cache.join("with_text").join("0001 - Cached Comic.png");
        assert!(!rendered.exists());
    }

    #[test]
    fn cache_health_reports_healthy_cache() {
        let sandbox = Sandbox::new();
        let cache = sandbox.xkcd_cache();
        fs::create_dir_all(cache.join("latest")).unwrap();
        fs::write(cache.join("latest").join("keep"), 42u32.to_le_bytes()).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("0042 - Healthy Comic.png"), "cached").unwrap();
        fs::create_dir_all(cache.join("metadata")).unwrap();
        fs::write(
            cache.join("metadata").join("0042.json"),
            "{\"img\":\"https://imgs.xkcd.com/comics/healthy.png\",\"title\":\"Healthy Comic\",\"alt\":\"Alt text\",\"num\":42}",
        )
        .unwrap();
        fs::create_dir_all(cache.join("with_text")).unwrap();
        fs::write(
            cache.join("with_text").join("0042 - Healthy Comic.png"),
            "rendered",
        )
        .unwrap();

        let assert = sandbox
            .command()
            .args(["cache", "health"])
            .assert()
            .success();

        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("healthy: yes"));
        assert!(stdout.contains("latest marker: ok (42)"));
        assert!(stdout.contains("images: 1 valid, 0 invalid"));
        assert!(stdout.contains("metadata: 1 valid, 0 invalid"));
        assert!(stdout.contains("rendered: 1 valid, 0 invalid"));
    }

    #[test]
    fn cache_health_reports_invalid_cache() {
        let sandbox = Sandbox::new();
        let cache = sandbox.xkcd_cache();
        fs::create_dir_all(cache.join("latest")).unwrap();
        fs::write(cache.join("latest").join("keep"), [1, 2, 3]).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("oops.txt"), "cached").unwrap();
        fs::create_dir_all(cache.join("metadata")).unwrap();
        fs::write(cache.join("metadata").join("oops.json"), "{").unwrap();
        fs::create_dir_all(cache.join("with_text")).unwrap();
        fs::write(cache.join("with_text").join("broken.png"), "rendered").unwrap();
        fs::write(cache.join(".leftover.tmp"), "").unwrap();

        let assert = sandbox
            .command()
            .args(["cache", "health"])
            .assert()
            .failure();

        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("healthy: no"));
        assert!(stdout.contains("latest marker: invalid"));
        assert!(stdout.contains("warning: invalid images entry at oops.txt"));
        assert!(stdout.contains("warning: invalid metadata entry at metadata/oops.json"));
        assert!(stdout.contains("warning: invalid rendered entry at with_text/broken.png"));
        assert!(stdout.contains("warning: staged file left behind at .leftover.tmp"));
    }

    fn test_path(bin: &Path) -> OsString {
        let mut path = OsString::new();
        path.push(bin.as_os_str());
        if let Some(existing) = std::env::var_os("PATH") {
            path.push(":");
            path.push(existing);
        }
        path
    }

    fn write_script(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }
}
