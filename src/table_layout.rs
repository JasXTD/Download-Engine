//! Stable column identities and validated, backward-compatible table preferences.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Column {
    Number,
    Name,
    Status,
    Progress,
    Size,
    Speed,
    Eta,
    Added,
    Actions,
}
impl Column {
    pub const ALL: [Self; 9] = [
        Self::Number,
        Self::Name,
        Self::Status,
        Self::Progress,
        Self::Size,
        Self::Speed,
        Self::Eta,
        Self::Added,
        Self::Actions,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Number => "#",
            Self::Name => "File name",
            Self::Status => "Status",
            Self::Progress => "Progress",
            Self::Size => "Size",
            Self::Speed => "Speed",
            Self::Eta => "Time left",
            Self::Added => "Added",
            Self::Actions => "Actions",
        }
    }
    pub fn minimum(self) -> u16 {
        match self {
            Self::Number => 30,
            Self::Name => 140,
            Self::Status => 90,
            Self::Progress => 94,
            Self::Size | Self::Speed | Self::Eta => 70,
            Self::Added => 100,
            Self::Actions => 88,
        }
    }
    pub fn default_width(self) -> u16 {
        match self {
            Self::Number => 30,
            Self::Name => 140,
            Self::Status => 166,
            Self::Progress => 120,
            Self::Size => 98,
            Self::Speed => 88,
            Self::Eta => 78,
            Self::Added => 104,
            Self::Actions => 88,
        }
    }
    pub fn visibility_index(self) -> Option<usize> {
        match self {
            Self::Number => Some(0),
            Self::Status => Some(1),
            Self::Progress => Some(2),
            Self::Size => Some(3),
            Self::Speed => Some(4),
            Self::Eta => Some(5),
            Self::Added => Some(6),
            Self::Name | Self::Actions => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TableLayout {
    pub order: [Column; 9],
    /// Logical pixels, indexed by identity, not display position. None uses auto sizing.
    pub widths: [Option<u16>; 9],
}
impl Default for TableLayout {
    fn default() -> Self {
        Self {
            order: [
                Column::Number,
                Column::Name,
                Column::Size,
                Column::Status,
                Column::Progress,
                Column::Speed,
                Column::Eta,
                Column::Added,
                Column::Actions,
            ],
            widths: [None; 9],
        }
    }
}
impl TableLayout {
    pub const MAX_WIDTH: u16 = 1200;
    pub fn validate(self) -> Result<()> {
        let mut seen = [false; 9];
        for column in self.order {
            ensure!(
                !seen[column as usize],
                "A table column appears more than once"
            );
            seen[column as usize] = true;
        }
        for column in Column::ALL {
            if let Some(width) = self.widths[column as usize] {
                ensure!(
                    (column.minimum()..=Self::MAX_WIDTH).contains(&width),
                    "Invalid table column width"
                );
            }
        }
        Ok(())
    }
    pub fn move_column(&mut self, source: Column, target: Column, before: bool) {
        if source == target {
            return;
        }
        let from = self
            .order
            .iter()
            .position(|c| *c == source)
            .expect("Validated column order");
        let target_index = self
            .order
            .iter()
            .position(|c| *c == target)
            .expect("Validated column order");
        let to = target_index + usize::from(!before) - usize::from(from < target_index);
        if from < to {
            self.order[from..=to].rotate_left(1);
        } else if to < from {
            self.order[to..=from].rotate_right(1);
        }
    }
}
