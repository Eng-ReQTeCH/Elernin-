use std::{collections::HashSet, fs::File, sync::LazyLock, time::Duration};

use anyhow::{anyhow, bail, ensure, Context, Result};
use reqwest::{
    blocking::{Client, Response},
    header, redirect, StatusCode, Url,
};
use scraper::{ElementRef, Html, Selector};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::library::{copy_response, Course, Material};

const HOST: &str = "learn1.bue.edu.eg";
const LOGIN: &str = "/login/index.php";
const DASHBOARD: &str = "/my/";
const MOBILE_TOKEN: &str = "/login/token.php";
const REST: &str = "/webservice/rest/server.php";
const AJAX: &str = "/lib/ajax/service.php";
const TILE_CONTENT: &str = "format_tiles_get_single_section_page_html";

pub struct Credentials {
    pub username: String,
    pub password: String,
}

pub struct Portal {
    client: Client,
    base: Url,
    token: Option<String>,
    browser_session: bool,
}

#[derive(Deserialize)]
struct MoodleCourse {
    id: i64,
    fullname: String,
}

#[derive(Deserialize)]
struct Section {
    name: String,
    modules: Vec<Activity>,
}

#[derive(Deserialize)]
struct Activity {
    id: i64,
    name: String,
    #[serde(default)]
    contents: Vec<RemoteFile>,
}

#[derive(Deserialize)]
struct RemoteFile {
    #[serde(rename = "type")]
    kind: String,
    filename: String,
    fileurl: Option<String>,
    filesize: Option<u64>,
    timemodified: Option<u64>,
}

#[derive(Default)]
pub struct ModuleFiles {
    pub files: Vec<Material>,
    pub issues: Vec<String>,
}

#[derive(Default)]
struct WebContents {
    files: Vec<Material>,
    sections: Vec<(String, Url)>,
    folders: Vec<(String, Url)>,
}

impl Portal {
    pub fn new() -> Result<Self> {
        Self::with_base(address("/"))
    }

    fn with_base(base: Url) -> Result<Self> {
        let origin = base.origin();
        let client = Client::builder()
            .user_agent("Elearnin/0.1")
            .connect_timeout(Duration::from_secs(12))
            .timeout(Duration::from_secs(90))
            .cookie_store(true)
            .redirect(redirect::Policy::custom(move |attempt| {
                if attempt.url().origin() == origin && attempt.previous().len() < 10 {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .build()?;
        Ok(Self {
            client,
            base,
            token: None,
            browser_session: false,
        })
    }

    fn endpoint(&self, path: &str) -> Url {
        self.base.join(path).expect("valid BUE endpoint")
    }

    pub fn courses(&mut self, credentials: &Credentials) -> Result<Vec<Course>> {
        if !self.browser_session && self.token.is_none() {
            self.token = self.mobile_token(credentials)?;
        }
        if self.token.is_some() {
            let mut result = self.mobile_courses();
            if result.as_ref().is_err_and(|error| error.is::<Expired>()) {
                self.token = self.mobile_token(credentials)?;
                result = if self.token.is_some() {
                    self.mobile_courses()
                } else {
                    Err(UnavailableApi.into())
                };
            }
            match result {
                Ok(courses) => return Ok(courses),
                Err(error) if error.is::<UnavailableApi>() => self.token = None,
                Err(error) => return Err(error),
            }
        }
        self.web_courses(credentials)
    }

    fn mobile_token(&self, credentials: &Credentials) -> Result<Option<String>> {
        let response = self
            .client
            .post(self.endpoint(MOBILE_TOKEN))
            .form(&[
                ("username", credentials.username.as_str()),
                ("password", credentials.password.as_str()),
                ("service", "moodle_mobile_app"),
            ])
            .send()
            .map_err(network)?;
        if response.status().is_server_error() || response.status() == 429 {
            bail!("BUE is temporarily unavailable. Saved materials are ready to open.");
        }
        if !response.status().is_success() {
            return Ok(None);
        }
        let value: Value = match response.json() {
            Ok(value) => value,
            Err(error) if error.is_decode() => return Ok(None),
            Err(error) => return Err(network(error)),
        };
        if matches!(
            value["errorcode"].as_str(),
            Some("invalidlogin" | "loginfailed")
        ) {
            return Err(SignInRejected.into());
        }
        Ok(value["token"]
            .as_str()
            .filter(|token| !token.is_empty())
            .map(str::to_owned))
    }

    fn api(&self, function: &str, params: &[(&str, &str)]) -> Result<Value> {
        let token = self.token.as_deref().context("Missing Moodle session")?;
        let mut form = vec![
            ("wstoken", token),
            ("wsfunction", function),
            ("moodlewsrestformat", "json"),
        ];
        form.extend_from_slice(params);
        let response = self
            .client
            .post(self.endpoint(REST))
            .form(&form)
            .send()
            .map_err(network)?;
        if matches!(
            response.status(),
            StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED | StatusCode::FORBIDDEN
        ) {
            return Err(UnavailableApi.into());
        }
        let response = checked(Ok(response), "requesting course data from the Moodle API")?;
        let value: Value = match response.json() {
            Ok(value) => value,
            Err(error) if error.is_decode() => return Err(UnavailableApi.into()),
            Err(error) => return Err(network(error)),
        };
        match value["errorcode"].as_str() {
            Some("invalidtoken") => Err(Expired.into()),
            Some(
                "accessexception"
                | "servicenotavailable"
                | "wsfunctionnotavailable"
                | "webservicenotavailable",
            ) => Err(UnavailableApi.into()),
            Some(_) => bail!("BUE could not return the requested course information"),
            None => Ok(value),
        }
    }

    fn mobile_courses(&self) -> Result<Vec<Course>> {
        let site = self.api("core_webservice_get_site_info", &[])?;
        let user_id = site["userid"]
            .as_i64()
            .context("BUE did not return a student ID")?
            .to_string();
        let courses: Vec<MoodleCourse> = serde_json::from_value(
            self.api("core_enrol_get_users_courses", &[("userid", &user_id)])?,
        )?;
        Ok(courses.into_iter().map(course).collect())
    }

    fn web_login(&mut self, credentials: &Credentials) -> Result<()> {
        let page = checked(
            self.client.get(self.endpoint(LOGIN)).send(),
            "loading the sign-in form",
        )?
        .text()
        .map_err(network)?;
        let login = Html::parse_document(&page);
        let field = Selector::parse("input[name=logintoken]").unwrap();
        let token = login
            .select(&field)
            .next()
            .and_then(|input| input.attr("value"))
            .context("BUE returned an unfamiliar sign-in page")?;
        let response = checked(
            self.client
                .post(self.endpoint(LOGIN))
                .form(&[
                    ("username", credentials.username.as_str()),
                    ("password", credentials.password.as_str()),
                    ("logintoken", token),
                ])
                .send(),
            "submitting the sign-in form",
        )?;
        if response.url().path().starts_with("/login/") {
            return Err(SignInRejected.into());
        }
        self.browser_session = true;
        Ok(())
    }

    fn web_courses(&mut self, credentials: &Credentials) -> Result<Vec<Course>> {
        let response = checked(
            self.client.get(self.endpoint(DASHBOARD)).send(),
            "opening the BUE dashboard",
        )?;
        let response = if response.url().path().starts_with("/login/") {
            self.web_login(credentials)?;
            checked(
                self.client.get(self.endpoint(DASHBOARD)).send(),
                "opening the BUE dashboard after sign-in",
            )?
        } else {
            response
        };
        ensure!(
            !response.url().path().starts_with("/login/"),
            "BUE did not keep the new session"
        );
        self.browser_session = true;
        let body = response.text().map_err(network)?;
        let key = session_key(&body).context("Could not read the BUE course dashboard")?;
        let mut courses = Vec::new();
        let mut seen = HashSet::new();
        for offset in (0..2000).step_by(50) {
            let mut data = self.ajax(
                key,
                "core_course_get_enrolled_courses_by_timeline_classification",
                json!({"classification": "all", "limit": 50, "offset": offset, "sort": "fullname"}),
                "listing enrolled modules",
            )?;
            let batch: Vec<MoodleCourse> = serde_json::from_value(data["courses"].take())?;
            let count = batch.len();
            let before = courses.len();
            courses.extend(
                batch
                    .into_iter()
                    .filter(|entry| seen.insert(entry.id))
                    .map(course),
            );
            if count < 50 || courses.len() == before {
                return Ok(courses);
            }
        }
        bail!("BUE returned more modules than Elearnin’ could read in one sync")
    }

    fn ajax(&self, key: &str, method: &str, args: Value, action: &str) -> Result<Value> {
        let response = checked(
            self.client
                .post(self.endpoint(AJAX))
                .query(&[("sesskey", key), ("info", method)])
                .json(&json!([{"index": 0, "methodname": method, "args": args}]))
                .send(),
            action,
        )?;
        if response.url().path().starts_with("/login/") {
            return Err(Expired.into());
        }
        let mut value: Value = response.json().map_err(network)?;
        let result = &value[0];
        if [&value, result, &result["exception"]].iter().any(|entry| {
            matches!(
                entry["errorcode"].as_str(),
                Some("invalidsesskey" | "requireloginerror")
            )
        }) {
            return Err(Expired.into());
        }
        ensure!(result["error"] == false, "BUE could not finish {action}");
        Ok(value[0]["data"].take())
    }

    pub fn materials(&self, course: i64) -> Result<ModuleFiles> {
        if self.token.is_some() {
            return Ok(ModuleFiles {
                files: self.mobile_materials(course)?,
                issues: Vec::new(),
            });
        }
        let mut url = self.endpoint("/course/view.php");
        url.query_pairs_mut().append_pair("id", &course.to_string());
        let page = self.course_page(url.clone())?;
        let WebContents {
            files,
            sections,
            mut folders,
        } = web_materials(&page, &url, "Course materials", false);
        let mut result = ModuleFiles {
            files,
            issues: Vec::new(),
        };
        let key = session_key(&page);
        // Tile HTML can be absent from the landing page even when the tile is visible.
        for (section, section_url) in sections {
            let loaded = self.section_contents(course, key, &section_url);
            match loaded {
                Ok(contents) => {
                    let mut parsed = web_materials(&contents, &section_url, &section, true);
                    result.files.append(&mut parsed.files);
                    folders.append(&mut parsed.folders);
                }
                Err(error) if error.is::<Expired>() => return Err(error),
                Err(error) => result.issues.push(format!("{section}: {error}")),
            }
        }
        let mut seen_folders = HashSet::new();
        for (section, folder) in folders {
            if !seen_folders.insert(folder.clone()) {
                continue;
            }
            match self.course_page(folder.clone()) {
                Ok(page) => result
                    .files
                    .extend(web_materials(&page, &folder, &section, false).files),
                Err(error) if error.is::<Expired>() => return Err(error),
                Err(error) => result.issues.push(format!("{section} folder: {error}")),
            }
        }
        // Prefer the week label when a file also appears on the landing page.
        let mut seen = HashSet::new();
        result.files.reverse();
        result.files.retain(|item| seen.insert(item.id.clone()));
        result.files.reverse();
        Ok(result)
    }

    fn section_contents(&self, course: i64, key: Option<&str>, url: &Url) -> Result<String> {
        if let (Some(key), Some(number)) = (key, tile_number(url)) {
            match self.ajax(
                key,
                TILE_CONTENT,
                json!({"courseid": course, "sectionid": number}),
                "loading a week tile",
            ) {
                Ok(data) => {
                    if let Some(html) = data["html"].as_str().filter(|html| !html.trim().is_empty())
                    {
                        return Ok(html.to_owned());
                    }
                }
                Err(error) if error.is::<Expired>() => return Err(error),
                Err(_) => (),
            }
        }
        self.course_page(url.clone())
    }

    fn course_page(&self, url: Url) -> Result<String> {
        let response = checked(self.client.get(url).send(), "opening a module page")?;
        if response.url().path().starts_with("/login/") {
            return Err(Expired.into());
        }
        response.text().map_err(network)
    }

    fn mobile_materials(&self, course: i64) -> Result<Vec<Material>> {
        let sections: Vec<Section> = serde_json::from_value(self.api(
            "core_course_get_contents",
            &[("courseid", &course.to_string())],
        )?)?;
        let mut materials = Vec::new();
        for section in sections {
            for activity in section.modules {
                for file in activity.contents {
                    if file.kind != "file" || !supported_file(&file.filename) {
                        continue;
                    }
                    let Some(url) = file.fileurl.as_deref().and_then(allowed_url) else {
                        continue;
                    };
                    if !url.path().starts_with("/webservice/pluginfile.php/")
                        && !url.path().starts_with("/pluginfile.php/")
                    {
                        continue;
                    }
                    materials.push(Material {
                        id: format!("{}:{:x}", activity.id, Sha256::digest(url.path())),
                        section: section.name.clone(),
                        title: activity.name.clone(),
                        filename: file.filename,
                        url,
                        size: file.filesize,
                        revision: file
                            .timemodified
                            .map(|time| format!("{time}:{:?}", file.filesize)),
                    });
                }
            }
        }
        Ok(materials)
    }

    pub fn download(&self, item: &Material, output: &mut File) -> Result<Option<String>> {
        let mut url = item.url.clone();
        if let Some(token) = &self.token {
            if let Some(path) = url.path().strip_prefix("/pluginfile.php/") {
                url.set_path(&format!("/webservice/pluginfile.php/{path}"));
            }
            url.query_pairs_mut().append_pair("token", token);
        }
        let response = checked(self.client.get(url).send(), "downloading a study file")?;
        if response.url().path().starts_with("/login/") {
            return Err(Expired.into());
        }
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        ensure!(
            !content_type.starts_with("text/html") && !content_type.starts_with("application/json"),
            "BUE returned a page in place of the study file"
        );
        let filename = response
            .url()
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .filter(|name| supported_file(name))
            .map(str::to_owned);
        ensure!(
            supported_file(&item.filename) || filename.is_some(),
            "BUE returned an unsupported study file"
        );
        copy_response(response, output)?;
        Ok(filename)
    }
}

#[derive(Debug)]
pub struct Expired;
impl std::fmt::Display for Expired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("The BUE session expired")
    }
}
impl std::error::Error for Expired {}

#[derive(Debug)]
pub struct SignInRejected;
impl std::fmt::Display for SignInRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BUE did not accept automatic sign-in. Check the saved password and any prompt on the BUE website.")
    }
}
impl std::error::Error for SignInRejected {}

#[derive(Debug)]
struct UnavailableApi;
impl std::fmt::Display for UnavailableApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BUE’s Moodle API is unavailable")
    }
}
impl std::error::Error for UnavailableApi {}

fn address(path: &str) -> Url {
    format!("https://{HOST}{path}")
        .parse()
        .expect("valid BUE URL")
}

fn same_origin(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some(HOST)
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
}

fn allowed_url(raw: &str) -> Option<Url> {
    let url = address("/").join(raw).ok()?;
    same_origin(&url).then_some(url)
}

fn network(error: reqwest::Error) -> anyhow::Error {
    if error.is_decode() {
        anyhow!("BUE returned an unexpected response")
    } else {
        anyhow!("Could not reach BUE. Saved materials are ready to open.")
    }
}

fn checked(response: reqwest::Result<Response>, action: &str) -> Result<Response> {
    let response = response.map_err(network)?;
    ensure!(
        response.status().is_success(),
        "BUE returned HTTP {} while {action}",
        response.status().as_u16(),
    );
    Ok(response)
}

fn course(entry: MoodleCourse) -> Course {
    Course {
        id: entry.id,
        name: entry.fullname,
    }
}

fn session_key(html: &str) -> Option<&str> {
    let value = html
        .split_once("\"sesskey\"")?
        .1
        .trim_start()
        .strip_prefix(':')?
        .trim_start()
        .strip_prefix('"')?;
    let key = value.split_once('"')?.0;
    (!key.is_empty() && key.bytes().all(|byte| byte.is_ascii_alphanumeric())).then_some(key)
}

fn supported_file(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| {
        matches!(
            ext.to_ascii_lowercase().as_str(),
            "pdf"
                | "ppt"
                | "pptx"
                | "doc"
                | "docx"
                | "xls"
                | "xlsx"
                | "txt"
                | "csv"
                | "odt"
                | "ods"
                | "odp"
                | "png"
                | "jpg"
                | "jpeg"
                | "zip"
                | "mp3"
                | "mp4"
        )
    })
}

fn tile_number(url: &Url) -> Option<u32> {
    if url.path() != "/course/view.php" {
        return None;
    }
    url.query_pairs()
        .find(|(key, _)| key == "section")?
        .1
        .parse()
        .ok()
        .filter(|&n| n > 0)
}

fn web_materials(html: &str, base: &Url, fallback_section: &str, fragment: bool) -> WebContents {
    static LINKS: LazyLock<Selector> = LazyLock::new(|| Selector::parse("a[href]").unwrap());
    static MAIN: LazyLock<Selector> =
        LazyLock::new(|| Selector::parse("#region-main, main, [role=main]").unwrap());
    static TILE: LazyLock<Selector> = LazyLock::new(|| {
        Selector::parse(
        "li.tile-clickable[data-section], .tile[data-section]:not(.tile-restricted):not(.tile-hidden)"
    ).unwrap()
    });
    static SECTION: LazyLock<Selector> =
        LazyLock::new(|| Selector::parse(".sectionname, .section-title, .tile-textinner").unwrap());
    static ACTIVITY: LazyLock<Selector> =
        LazyLock::new(|| Selector::parse(".instancename, .activityname").unwrap());
    let html = if fragment {
        Html::parse_fragment(html)
    } else {
        Html::parse_document(html)
    };
    let scope = if fragment {
        html.root_element()
    } else {
        html.select(&MAIN)
            .next()
            .unwrap_or_else(|| html.root_element())
    };
    let mut contents = WebContents::default();
    let mut seen_sections = HashSet::new();
    let course = base
        .query_pairs()
        .find(|(key, _)| key == "id")
        .map(|(_, value)| value.into_owned());
    if base.path() == "/course/view.php" && tile_number(base).is_none() {
        if let Some(course) = &course {
            for tile in scope.select(&TILE) {
                let Some(number) = tile
                    .attr("data-section")
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|&n| n > 0)
                else {
                    continue;
                };
                let mut url = base.join("/course/view.php").expect("valid section path");
                url.query_pairs_mut()
                    .append_pair("id", course)
                    .append_pair("section", &number.to_string());
                let title = tile
                    .select(&SECTION)
                    .next()
                    .map(label)
                    .filter(|title| !title.is_empty())
                    .unwrap_or_else(|| format!("Week {number}"));
                if seen_sections.insert(url.clone()) {
                    contents.sections.push((title, url));
                }
            }
        }
    }
    for anchor in scope.select(&LINKS) {
        let Some(mut url) = anchor
            .attr("href")
            .and_then(|href| base.join(href).ok())
            .filter(|url| {
                url.origin() == base.origin()
                    && url.username().is_empty()
                    && url.password().is_none()
            })
        else {
            continue;
        };
        url.set_fragment(None);
        let title = anchor
            .select(&ACTIVITY)
            .next()
            .map_or_else(|| label(anchor), label);
        if !fragment {
            if let Some(section) = section_link(base, &url, course.as_deref()) {
                if !title.is_empty() && seen_sections.insert(section.clone()) {
                    contents.sections.push((title, section));
                }
                continue;
            }
        }
        if url.path() == "/mod/folder/view.php"
            && url
                .query_pairs()
                .any(|(key, value)| key == "id" && value.parse::<u64>().is_ok())
        {
            contents.folders.push((fallback_section.to_owned(), url));
            continue;
        }
        let tiles_resource = url.path() == "/course/format/tiles/mod_view.php"
            && url
                .query_pairs()
                .any(|(key, value)| key == "modtype" && value == "resource");
        let (id, filename) = if url.path() == "/mod/resource/view.php" || tiles_resource {
            let Some((_, id)) = url.query_pairs().find(|(name, _)| name == "id") else {
                continue;
            };
            let filename = anchor
                .ancestors()
                .filter_map(ElementRef::wrap)
                .take(5)
                .find_map(|element| element.attr("data-filename"))
                .map(str::to_owned)
                .unwrap_or_else(|| title.clone());
            let id = id.into_owned();
            if tiles_resource {
                url = base
                    .join("/mod/resource/view.php")
                    .expect("valid resource path");
                url.query_pairs_mut().append_pair("id", &id);
            }
            url.query_pairs_mut().append_pair("redirect", "1");
            (format!("resource:{id}"), filename)
        } else if url.path().starts_with("/pluginfile.php/") {
            let filename = url
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .unwrap_or("")
                .to_owned();
            if !supported_file(&filename) {
                continue;
            }
            (format!("file:{:x}", Sha256::digest(url.path())), filename)
        } else {
            continue;
        };
        let section = anchor
            .ancestors()
            .filter_map(ElementRef::wrap)
            .find(|element| {
                element
                    .value()
                    .classes()
                    .any(|name| name == "course-section" || name == "section")
            })
            .and_then(|element| element.select(&SECTION).next())
            .map(label)
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| fallback_section.to_owned());
        contents.files.push(Material {
            id,
            section,
            title: if title.is_empty() {
                filename.clone()
            } else {
                title
            },
            filename,
            url,
            revision: None,
            size: None,
        });
    }
    contents
}

fn section_link(base: &Url, url: &Url, course: Option<&str>) -> Option<Url> {
    let number = |name| {
        url.query_pairs()
            .find(|(key, _)| key == name)?
            .1
            .parse::<u64>()
            .ok()
            .filter(|&n| n > 0)
    };
    let mut section = base.join(url.path()).ok()?;
    match url.path() {
        "/course/section.php" => {
            section
                .query_pairs_mut()
                .append_pair("id", &number("id")?.to_string());
        }
        "/course/view.php" => {
            let course = course.filter(|id| {
                number("id").is_some_and(|linked| id.parse::<u64>().ok() == Some(linked))
            })?;
            let (key, value) = number("section")
                .map(|n| ("section", n))
                .or_else(|| number("sectionid").map(|n| ("sectionid", n)))?;
            section
                .query_pairs_mut()
                .append_pair("id", course)
                .append_pair(key, &value.to_string());
        }
        _ => return None,
    }
    Some(section)
}

fn label(element: ElementRef<'_>) -> String {
    element
        .text()
        .flat_map(str::split_whitespace)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        thread,
    };

    use super::*;

    #[test]
    fn falls_back_to_dashboard_and_signs_in_again_when_the_session_expires() -> Result<()> {
        let server = TcpListener::bind("127.0.0.1:0")?;
        let base = format!("http://{}/", server.local_addr()?).parse()?;
        let login_form = r#"<input name="logintoken" value="formtoken">"#;
        let dashboard = r#"<script>M.cfg = {"sesskey":"sessionkey"};</script>"#;
        let courses =
            r#"[{"error":false,"data":{"courses":[{"id":42,"fullname":"Engineering"}]}}]"#;
        let landing = r#"<main><script>M.cfg = {"sesskey":"sessionkey"};</script>
            <ul class="tiles"><li class="tile tile-clickable" data-section="1">Week 1</li>
            <li class="tile tile-clickable" data-section="12">Week 12</li></ul></main>"#;
        let tile = r#"[{"error":false,"data":{"html":"<a href='/pluginfile.php/7/notes.pdf'>Lecture</a>"}}]"#;
        let week = "<main><a href='/mod/folder/view.php?id=91'>Tutorials</a></main>";
        let folder = "<main><a href='/pluginfile.php/8/worksheet.pdf'>Worksheet</a></main>";
        let serve = thread::spawn(move || -> Result<()> {
            let initial = [
                (
                    "POST /login/token.php",
                    "200 OK",
                    "",
                    r#"{"token":"mobiletoken"}"#,
                ),
                ("POST /webservice/rest/server.php", "404 Not Found", "", ""),
            ];
            let web_sync = [
                (
                    "GET /my/",
                    "303 See Other",
                    "Location: /login/index.php\r\n",
                    "",
                ),
                ("GET /login/index.php", "200 OK", "", login_form),
                ("GET /login/index.php", "200 OK", "", login_form),
                (
                    "POST /login/index.php",
                    "303 See Other",
                    "Location: /my/\r\nSet-Cookie: MoodleSession=test; Path=/\r\n",
                    "",
                ),
                ("GET /my/", "200 OK", "", dashboard),
                ("GET /my/", "200 OK", "", dashboard),
                (
                    "POST /lib/ajax/service.php?sesskey=sessionkey",
                    "200 OK",
                    "",
                    courses,
                ),
            ];
            let downloads = [
                ("GET /course/view.php?id=42 ", "200 OK", "", landing),
                ("POST /lib/ajax/service.php?sesskey=sessionkey&info=format_tiles_get_single_section_page_html", "200 OK", "", tile),
                ("POST /lib/ajax/service.php?sesskey=sessionkey&info=format_tiles_get_single_section_page_html", "404 Not Found", "", ""),
                ("GET /course/view.php?id=42&section=12 ", "200 OK", "", week),
                ("GET /mod/folder/view.php?id=91 ", "200 OK", "", folder),
                ("GET /pluginfile.php/7/notes.pdf ", "200 OK", "Content-Type: application/pdf\r\n", "lecture"),
                ("GET /pluginfile.php/8/worksheet.pdf ", "200 OK", "Content-Type: application/pdf\r\n", "tutorial"),
            ];
            for (expected, status, headers, body) in initial
                .into_iter()
                .chain(web_sync)
                .chain(downloads)
                .chain(web_sync)
            {
                let (mut stream, _) = server.accept()?;
                stream.set_read_timeout(Some(Duration::from_secs(3)))?;
                let mut reader = BufReader::new(&mut stream);
                let mut line = String::new();
                reader.read_line(&mut line)?;
                assert!(
                    line.starts_with(expected),
                    "expected {expected}, got {line}"
                );
                let mut length = 0;
                let mut signed_in = false;
                loop {
                    line.clear();
                    if reader.read_line(&mut line)? == 0 || line == "\r\n" {
                        break;
                    }
                    let line = line.to_ascii_lowercase();
                    if let Some(value) = line.strip_prefix("content-length:") {
                        length = value.trim().parse()?;
                    }
                    signed_in |= line.starts_with("cookie: moodlesession=test");
                }
                let mut request = vec![0; length];
                reader.read_exact(&mut request)?;
                if expected == "POST /login/index.php" {
                    let request = String::from_utf8(request)?;
                    assert!(
                        request.contains("username=student&password=example&logintoken=formtoken")
                    );
                }
                if expected.starts_with("POST /lib/ajax/") {
                    assert!(signed_in);
                }
                write!(stream, "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())?;
            }
            Ok(())
        });
        let mut portal = Portal::with_base(base)?;
        let credentials = Credentials {
            username: "student".into(),
            password: "example".into(),
        };
        for attempt in 0..2 {
            let courses = portal.courses(&credentials)?;
            assert_eq!(courses.len(), 1);
            assert_eq!(courses[0].id, 42);
            if attempt == 0 {
                let materials = portal.materials(42)?;
                assert!(materials.issues.is_empty());
                assert_eq!(materials.files.len(), 2);
                let root = tempfile::tempdir()?;
                let mut library = crate::library::Library::open(root.path())?;
                library.save_course(&courses[0])?;
                for file in materials.files {
                    library.save_material(42, &file, |output| portal.download(&file, output))?;
                }
                let files = library.files(42)?;
                assert_eq!(
                    (&*files[0].section, &*files[1].section),
                    ("Week 1", "Week 12")
                );
                assert_eq!(std::fs::read(&files[0].path)?, b"lecture");
                assert_eq!(std::fs::read(&files[1].path)?, b"tutorial");
            }
        }
        serve.join().expect("mock BUE server panicked")?;
        Ok(())
    }

    #[test]
    fn reads_moodle_course_files_and_keeps_requests_on_bue() {
        let page = r#"
            <main><section class="course-section">
              <h3 class="sectionname">Week 1</h3>
              <a href="/mod/resource/view.php?id=92"><span class="instancename">Lecture 1</span></a>
              <a href="https://learn1.bue.edu.eg/pluginfile.php/7/mod_folder/content/0/Tutorial.pdf">Tutorial</a>
              <a href="https://other.example/pluginfile.php/7/Handout.pdf">External</a>
              <a href="/course/section.php?id=5">Week 2</a>
            </section></main>"#;
        let base = address("/course/view.php?id=42");
        let WebContents {
            files, sections, ..
        } = web_materials(page, &base, "Course materials", false);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].title, "Lecture 1");
        assert_eq!(files[0].section, "Week 1");
        assert!(files[0]
            .url
            .query_pairs()
            .any(|(key, value)| key == "redirect" && value == "1"));
        assert_eq!(files[1].filename, "Tutorial.pdf");
        assert_eq!(sections[0].0, "Week 2");
    }

    #[test]
    fn reads_landing_page_tiles_and_standalone_week_content() {
        let base = address("/course/view.php?id=42");
        let landing = r#"<main><ul class="tiles">
            <li class="tile tile-clickable" data-section="2"><a class="tile-link" href="view.php?id=42&amp;section=2"><span class="tile-textinner">Week 2</span></a></li>
            <li class="tile tile-clickable" data-section="12"><span class="tile-textinner">Week 12</span></li>
            <li class="tile tile-restricted" data-section="13"><span>Week 13</span></li>
            </ul></main>"#;
        let sections = web_materials(landing, &base, "Course materials", false).sections;
        assert_eq!(sections.len(), 2);
        assert_eq!(
            (&*sections[0].0, tile_number(&sections[0].1)),
            ("Week 2", Some(2))
        );
        assert_eq!(
            (&*sections[1].0, tile_number(&sections[1].1)),
            ("Week 12", Some(12))
        );

        let response = r#"<div class="content">
            <a href="../mod/resource/view.php?id=92">Lecture slides</a>
            <a href="../mod/folder/view.php?id=93">Tutorials</a>
            </div>"#;
        let contents = web_materials(response, &sections[1].1, "Week 12", true);
        assert_eq!(contents.files.len(), 1);
        assert_eq!(contents.files[0].section, "Week 12");
        assert_eq!(contents.folders[0].0, "Week 12");
    }
}
