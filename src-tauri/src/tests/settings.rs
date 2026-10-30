//! Portable mode, startup theme and window size.

use super::*;

#[test]
fn portable_mode_requires_a_data_directory_next_to_the_executable() {
    let dir = temp_dir("portable");

    assert_eq!(portable_data_dir_in(&dir), None);

    std::fs::write(dir.join("data"), b"").expect("write plain file");
    // A plain file named `data` is not the marker.
    assert_eq!(portable_data_dir_in(&dir), None);

    std::fs::remove_file(dir.join("data")).expect("remove file");
    std::fs::create_dir(dir.join("data")).expect("create data dir");
    assert_eq!(portable_data_dir_in(&dir), Some(dir.join("data")));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_startup_theme_survives_a_restart_and_defaults_to_dark() {
    let dir = temp_dir("appearance");
    let path = dir.join("appearance.json");

    // Nothing saved yet, and an unreadable file, both mean the dark palette
    // the stylesheet starts from.
    assert_eq!(startup_theme_at(&path), Theme::Dark);
    std::fs::write(&path, b"not json").expect("write damaged file");
    assert_eq!(startup_theme_at(&path), Theme::Dark);

    save_startup_theme_at(&path, Theme::Light).expect("save light");
    assert_eq!(startup_theme_at(&path), Theme::Light);

    // The front end saves on every start, so a call that changes nothing
    // leaves the file alone: what is on disk is still the untouched original.
    std::fs::write(&path, br#"{"theme":"light","from":"a later build"}"#)
        .expect("write file with an unknown field");
    save_startup_theme_at(&path, Theme::Light).expect("save light again");
    assert!(
        std::fs::read_to_string(&path)
            .expect("read file")
            .contains("a later build")
    );

    save_startup_theme_at(&path, Theme::Dark).expect("save dark");
    assert_eq!(startup_theme_at(&path), Theme::Dark);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_theme_is_stored_under_the_name_the_front_end_uses() {
    let dir = temp_dir("appearance-names");
    let path = dir.join("appearance.json");

    // The front end writes these ids to its own storage (`ThemeMode` in
    // src/types.ts) and sends them here, so the two spellings have to agree
    // or a serialX user's window opens in the wrong background colour.
    // Each step is a different theme from the one before, since a save that
    // changes nothing leaves the file alone.
    for (theme, name) in [
        (Theme::SerialxDark, "serialx-dark"),
        (Theme::SerialxLight, "serialx-light"),
        (Theme::Light, "light"),
        (Theme::Dark, "dark"),
    ] {
        save_startup_theme_at(&path, theme).expect("save theme");
        assert!(
            std::fs::read_to_string(&path)
                .expect("read file")
                .contains(&format!("\"{name}\""))
        );
        assert_eq!(startup_theme_at(&path), theme);
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_window_size_survives_a_restart_beside_the_theme() {
    let dir = temp_dir("appearance-window");
    let path = dir.join("appearance.json");
    let size = WindowGeometry {
        width: 1280,
        height: 720,
        maximized: false,
    };

    // Nothing saved, a damaged file, and a file from a build that only
    // knew the theme all mean the configured size.
    assert_eq!(startup_window_at(&path), None);
    std::fs::write(&path, b"not json").expect("write damaged file");
    assert_eq!(startup_window_at(&path), None);
    std::fs::write(&path, br#"{"theme":"light"}"#).expect("write theme-only file");
    assert_eq!(startup_window_at(&path), None);
    assert_eq!(startup_theme_at(&path), Theme::Light);

    // The size joins the theme in the file without disturbing it, and the
    // theme's own save keeps the size.
    save_window_geometry_at(&path, size).expect("save size");
    assert_eq!(startup_window_at(&path), Some(size));
    assert_eq!(startup_theme_at(&path), Theme::Light);
    save_startup_theme_at(&path, Theme::Dark).expect("save dark");
    assert_eq!(startup_theme_at(&path), Theme::Dark);
    assert_eq!(startup_window_at(&path), Some(size));

    // Every launch reports the size it was created at: an unchanged save
    // leaves the file alone.
    std::fs::write(
        &path,
        br#"{"theme":"dark","window":{"width":1280,"height":720},"from":"a later build"}"#,
    )
    .expect("write file with an unknown field");
    save_window_geometry_at(&path, size).expect("save size again");
    let raw = std::fs::read_to_string(&path).expect("read file");
    assert!(raw.contains("a later build"), "{raw}");

    // Maximized is remembered on top of the size it restores to.
    let maximized = WindowGeometry {
        maximized: true,
        ..size
    };
    save_window_geometry_at(&path, maximized).expect("save maximized");
    assert_eq!(startup_window_at(&path), Some(maximized));

    // An empty size (a hand-edited file) is no size at all.
    std::fs::write(
        &path,
        br#"{"theme":"dark","window":{"width":0,"height":600}}"#,
    )
    .expect("write empty size");
    assert_eq!(startup_window_at(&path), None);

    std::fs::remove_dir_all(&dir).ok();
}
