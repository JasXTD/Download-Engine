//! What a site reader returns. It is checked before anything is shown, and it
//! becomes the same list the browser's Find videos on this page produces.
use crate::browser_video::{Item, Kind, Review};
use anyhow::{Result, anyhow, ensure};
use serde::{Deserialize, Serialize};

pub const MAX_ITEMS: usize = 1000;
pub const MAX_READING_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reading {
    #[serde(default)]
    pub title: Option<String>,
    pub items: Vec<ReadingItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadingItem {
    pub kind: Kind,
    pub url: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
}

impl Reading {
    /// The script's return value, as JSON. Lua has no separate empty-list type,
    /// so `items = {}` arrives as an empty object.
    pub fn from_value(mut value: serde_json::Value) -> Result<Self> {
        ensure!(
            !value.is_null(),
            "The site reader returned nothing. End it with: return {{ items = {{ … }} }}"
        );
        if value
            .get("items")
            .is_some_and(|items| items.as_object().is_some_and(|o| o.is_empty()))
        {
            value["items"] = serde_json::Value::Array(Vec::new());
        }
        let reading: Self = serde_json::from_value(value)
            .map_err(|error| anyhow!("The site reader returned an unexpected value: {error}"))?;
        reading.check()?;
        Ok(reading)
    }

    pub fn check(&self) -> Result<()> {
        ensure!(
            self.items.len() <= MAX_ITEMS,
            "A site reader can return at most {MAX_ITEMS} items"
        );
        ensure!(
            self.title.as_ref().is_none_or(|t| t.len() <= 2048),
            "The site reader's title is too long"
        );
        for item in &self.items {
            crate::model::validate_url(&item.url)?;
            ensure!(
                item.label.as_ref().is_none_or(|l| l.len() <= 2048)
                    && item.filename.as_ref().is_none_or(|f| f.len() <= 1024),
                "A site reader item's label or filename is too long"
            );
            ensure!(
                item.size
                    .is_none_or(|size| size > 0 && size <= 9_007_199_254_740_991),
                "A site reader item has an invalid size"
            );
            ensure!(
                item.kind != Kind::File || !crate::hls::is_playlist_url(&item.url),
                "Return .m3u8 and .mpd addresses as kind = \"hls\" or \"dash\", not \"file\""
            );
        }
        ensure!(
            serde_json::to_vec(self)?.len() <= MAX_READING_BYTES,
            "The site reader's result exceeds 1 MiB"
        );
        Ok(())
    }

    /// One stream and nothing else goes straight to the quality choice.
    pub fn single_stream(&self) -> Option<&ReadingItem> {
        match self.items.as_slice() {
            [item] if item.kind != Kind::File => Some(item),
            _ => None,
        }
    }

    /// The list the Video downloads window shows for `page`.
    pub fn review(&self, page: &str) -> Result<Review> {
        let page = crate::model::validate_url(page)?;
        let title = self
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| page.host_str().unwrap_or("Video page").to_owned());
        let items = self
            .items
            .iter()
            .map(|item| Item {
                url: item.url.clone(),
                filename: item
                    .filename
                    .clone()
                    .filter(|f| !f.trim().is_empty())
                    .unwrap_or_else(|| file_name(&item.url)),
                label: item.label.clone().unwrap_or_default(),
                kind: item.kind,
                size: item.size,
            })
            .collect();
        Review {
            public_only: true,
            page_url: page.to_string(),
            page_title: title,
            items,
            truncated: false,
        }
        .normalized()
    }
}

/// The last path segment of a link, or "video" when it has none.
fn file_name(url: &str) -> String {
    crate::model::validate_url(url)
        .ok()
        .and_then(|url| {
            url.path_segments()?
                .next_back()
                .filter(|s| !s.is_empty())
                .map(|s| {
                    percent_encoding::percent_decode_str(s)
                        .decode_utf8_lossy()
                        .into_owned()
                })
        })
        .unwrap_or_else(|| "video".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn returned_values_are_checked_and_become_a_review() {
        let reading = Reading::from_value(json!({
            "title": "Nature documentary",
            "items": [
                {"kind": "hls", "url": "https://cdn.example.com/v/master.m3u8", "label": "All qualities"},
                {"kind": "file", "url": "https://cdn.example.com/v/720%20p.mp4", "size": 84_200_000},
            ],
        }))
        .unwrap();
        assert!(reading.single_stream().is_none());
        let review = reading
            .review("https://www.example.com/watch/a1#t=3")
            .unwrap();
        assert_eq!(review.page_url, "https://www.example.com/watch/a1");
        assert_eq!(review.page_title, "Nature documentary");
        assert_eq!(review.items[0].filename, "master.m3u8");
        assert_eq!(review.items[1].filename, "720 p.mp4");
        assert_eq!(review.items[1].label, "");

        let single = Reading::from_value(
            json!({"items": [{"kind": "dash", "url": "https://cdn.example.com/a.mpd"}]}),
        )
        .unwrap();
        assert_eq!(
            single.single_stream().unwrap().url,
            "https://cdn.example.com/a.mpd"
        );
        let empty = Reading::from_value(json!({"items": {}})).unwrap();
        assert!(empty.items.is_empty());
        assert_eq!(
            empty.review("https://example.com/x").unwrap().page_title,
            "example.com"
        );

        for bad in [
            json!(null),
            json!("text"),
            json!({"title": "no items"}),
            json!({"items": [{"kind": "torrent", "url": "https://example.com/a"}]}),
            json!({"items": [{"kind": "file", "url": "file:///C:/secret.txt"}]}),
            json!({"items": [{"kind": "file", "url": "https://example.com/list.m3u8"}]}),
            json!({"items": [{"kind": "file", "url": "https://example.com/a", "size": 0}]}),
            json!({"items": [{"kind": "file", "url": "https://example.com/a", "extra": true}]}),
        ] {
            assert!(Reading::from_value(bad.clone()).is_err(), "{bad}");
        }
        let many =
            json!({"items": vec![json!({"kind": "file", "url": "https://example.com/a"}); 1001]});
        assert!(Reading::from_value(many).is_err());
    }
}
