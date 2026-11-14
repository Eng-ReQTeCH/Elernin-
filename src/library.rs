use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use tempfile::Builder;

#[derive(Clone)]
pub struct Course {
    pub id: i64,
    pub name: String,
}

pub struct Material {
    pub id: String,
    pub section: String,
    pub title: String,
    pub filename: String,
    pub url: reqwest::Url,
    pub revision: Option<String>,
    pub size: Option<u64>,
}

#[derive(Clone)]
pub struct SavedFile {
    pub id: String,
    pub section: String,
    pub title: String,
    pub filename: String,
    pub path: PathBuf,
}

pub struct Library {
    db: Connection,
    files: PathBuf,
}

impl Library {
    pub fn open(root: &Path) -> Result<Self> {
        let files = root.join("files");
        fs::create_dir_all(&files)?;
        let db = Connection::open(root.join("library.sqlite3"))?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS courses (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS materials (
                 course_id INTEGER NOT NULL REFERENCES courses(id),
                 id TEXT NOT NULL, section TEXT NOT NULL, title TEXT NOT NULL,
                 filename TEXT NOT NULL, local_name TEXT NOT NULL, revision TEXT,
                 PRIMARY KEY(course_id, id)
             );
             CREATE TABLE IF NOT EXISTS sync_info (id INTEGER PRIMARY KEY, last_success INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS preferences (name TEXT PRIMARY KEY, value INTEGER NOT NULL);",
        )?;
        Ok(Self { db, files })
    }

    pub fn courses(&self) -> Result<Vec<Course>> {
        let mut statement = self
            .db
            .prepare("SELECT id, name FROM courses ORDER BY name COLLATE NOCASE")?;
        let rows = statement.query_map([], |row| {
            Ok(Course {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn save_course(&self, course: &Course) -> Result<()> {
        self.db.execute(
            "INSERT INTO courses VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET name = excluded.name",
            params![course.id, course.name],
        )?;
        Ok(())
    }

    pub fn files(&self, course: i64) -> Result<Vec<SavedFile>> {
        let mut statement = self.db.prepare(
            "SELECT id, section, title, filename, local_name FROM materials WHERE course_id = ?1
             ORDER BY rowid",
        )?;
        let rows = statement.query_map([course], |row| {
            Ok(SavedFile {
                id: row.get(0)?,
                section: row.get(1)?,
                title: row.get(2)?,
                filename: row.get(3)?,
                path: self.files.join(row.get::<_, String>(4)?),
            })
        })?;
        let mut files: Vec<SavedFile> = rows.collect::<rusqlite::Result<_>>()?;
        files.sort_by_cached_key(|file| section_order(&file.section));
        Ok(files)
    }

    pub fn cached_path(root: &Path, course: i64, id: &str) -> Result<Option<PathBuf>> {
        let db = Connection::open_with_flags(
            root.join("library.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let local_name: Option<String> = db
            .query_row(
                "SELECT local_name FROM materials WHERE course_id = ?1 AND id = ?2",
                params![course, id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(local_name.map(|name| root.join("files").join(name)))
    }

    pub fn file_counts(&self) -> Result<std::collections::HashMap<i64, i64>> {
        let mut statement = self
            .db
            .prepare("SELECT course_id, count(*) FROM materials GROUP BY course_id")?;
        let counts = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(counts.collect::<rusqlite::Result<_>>()?)
    }

    pub fn dark_mode(&self) -> Result<bool> {
        Ok(self
            .db
            .query_row(
                "SELECT value FROM preferences WHERE name = 'dark_mode'",
                [],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(false))
    }

    pub fn set_dark_mode(&self, dark: bool) -> Result<()> {
        self.db.execute(
            "INSERT OR REPLACE INTO preferences VALUES ('dark_mode', ?1)",
            [dark],
        )?;
        Ok(())
    }

    pub fn last_sync(&self) -> Result<Option<i64>> {
        Ok(self
            .db
            .query_row(
                "SELECT last_success FROM sync_info WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn mark_synced(&self) -> Result<()> {
        self.db.execute(
            "INSERT OR REPLACE INTO sync_info VALUES (1, unixepoch())",
            [],
        )?;
        Ok(())
    }

    pub fn save_material(
        &mut self,
        course: i64,
        item: &Material,
        download: impl FnOnce(&mut fs::File) -> Result<Option<String>>,
    ) -> Result<()> {
        let previous: Option<(String, Option<String>)> = self
            .db
            .query_row(
                "SELECT local_name, revision FROM materials WHERE course_id = ?1 AND id = ?2",
                params![course, item.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((name, revision)) = &previous {
            if item.revision.is_some()
                && revision == &item.revision
                && self.files.join(name).is_file()
            {
                self.db.execute(
                    "UPDATE materials SET section = ?3, title = ?4, filename = ?5 WHERE course_id = ?1 AND id = ?2",
                    params![course, item.id, item.section, item.title, item.filename],
                )?;
                return Ok(());
            }
        }

        let mut file = Builder::new()
            .prefix("material-")
            .tempfile_in(&self.files)?;
        let downloaded_name = download(file.as_file_mut())?;
        let filename = downloaded_name.as_deref().unwrap_or(&item.filename);
        let suffix = Path::new(filename)
            .extension()
            .and_then(|value| value.to_str())
            .filter(|value| {
                !value.is_empty()
                    && value.len() <= 12
                    && value.bytes().all(|b| b.is_ascii_alphanumeric())
            })
            .map_or_else(String::new, |value| format!(".{value}"));
        let length = file.as_file().metadata()?.len();
        if item.size.is_some_and(|size| size != length) {
            anyhow::bail!("BUE returned an incomplete file");
        }
        file.as_file().sync_all()?;
        let path = file.path().with_extension(suffix.trim_start_matches('.'));
        file.persist_noclobber(&path)
            .context("keep downloaded file")?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .context("invalid local filename")?;
        let result = self.db.execute(
            "INSERT INTO materials VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(course_id, id) DO UPDATE SET section = excluded.section, title = excluded.title,
             filename = excluded.filename, local_name = excluded.local_name, revision = excluded.revision",
            params![course, item.id, item.section, item.title, filename, name, item.revision],
        );
        if let Err(error) = result {
            let _ = fs::remove_file(path);
            return Err(error.into());
        }
        if let Some((old, _)) = previous {
            let _ = fs::remove_file(self.files.join(old));
        }
        Ok(())
    }
}

fn section_order(section: &str) -> (u8, u32, String) {
    let label = section.trim().to_lowercase();
    if let Some(rest) = label.strip_prefix("week ") {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(week) = digits.parse() {
            return (0, week, label);
        }
    }
    (1, 0, label)
}

pub fn copy_response(
    mut response: reqwest::blocking::Response,
    output: &mut fs::File,
) -> Result<()> {
    let length = response.content_length();
    let copied = io::copy(&mut response, output)?;
    if length.is_some_and(|expected| expected != copied) {
        anyhow::bail!("BUE returned an incomplete file");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn sorts_numbered_weeks_naturally() {
        let mut sections = ["Week 12", "General", "Week 2", "Week 1"];
        sections.sort_by_cached_key(|section| section_order(section));
        assert_eq!(sections, ["Week 1", "Week 2", "Week 12", "General"]);
    }

    #[test]
    fn interrupted_update_keeps_previous_download() -> Result<()> {
        let root = tempfile::tempdir()?;
        let mut library = Library::open(root.path())?;
        library.save_course(&Course {
            id: 7,
            name: "Programming".into(),
        })?;
        let mut material = Material {
            id: "42".into(),
            section: "Week 1".into(),
            title: "Lecture".into(),
            filename: "slides.pdf".into(),
            url: "https://learn1.bue.edu.eg/file".parse()?,
            revision: Some("1".into()),
            size: Some(4),
        };
        library.save_material(7, &material, |file| {
            file.write_all(b"old!")?;
            Ok(None)
        })?;
        material.revision = Some("2".into());
        assert!(library
            .save_material(7, &material, |file| {
                file.write_all(b"new")?;
                Ok(None)
            })
            .is_err());
        drop(library);
        let library = Library::open(root.path())?;
        let saved = library.files(7)?;
        assert_eq!(fs::read(&saved[0].path)?, b"old!");
        assert_eq!(fs::read_dir(&library.files)?.count(), 1);
        Ok(())
    }

    #[test]
    fn open_uses_local_copy_during_download_and_after_replacement() -> Result<()> {
        let root = tempfile::tempdir()?;
        let mut library = Library::open(root.path())?;
        library.save_course(&Course {
            id: 7,
            name: "Programming".into(),
        })?;
        let mut material = Material {
            id: "42".into(),
            section: "Week 1".into(),
            title: "Lecture".into(),
            filename: "slides.pdf".into(),
            url: "https://learn1.bue.edu.eg/file".parse()?,
            revision: Some("1".into()),
            size: Some(4),
        };
        library.save_material(7, &material, |output| {
            output.write_all(b"old!")?;
            Ok(None)
        })?;
        let displayed = library.files(7)?.remove(0);
        material.revision = Some("2".into());
        library.save_material(7, &material, |output| {
            output.write_all(b"ne")?;
            crate::app::open_saved(root.path(), 7, &displayed, |path| {
                assert_eq!(fs::read(path)?, b"old!");
                Ok(())
            })?;
            output.write_all(b"w!")?;
            Ok(None)
        })?;
        assert!(!displayed.path.exists());
        crate::app::open_saved(root.path(), 7, &displayed, |path| {
            assert_eq!(fs::read(path)?, b"new!");
            Ok(())
        })
    }
}
