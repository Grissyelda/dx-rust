//! Readable things (InformationDevices: datacubes, books, newspapers): using one shows its text (looked up by
//! textTag in DeusExText.u) in a window until you use it again or walk away.

use crate::Player;
use bevy::prelude::*;
use glam::DVec3;

#[derive(Resource, Default)]
pub struct InfoDisplay {
    pub text: Option<String>,
    pub tag: String,
    pub at: DVec3,
    pkg: Option<unreal_pkg::Package>,
    tried: bool,
}

#[derive(Component)]
pub struct InfoPanel;
#[derive(Component)]
pub struct InfoText;

/// Strip the text markup: <COMMENT>..</COMMENT> goes, <P> is a new paragraph, other tags (<DC=..>, <B>) are dropped.
pub fn format_text(raw: &str) -> String {
    let mut s = raw.replace('\0', "").replace('\r', "");
    while let (Some(a), Some(b)) = (s.find("<COMMENT>"), s.find("</COMMENT>")) {
        if b < a {
            break;
        }
        s.replace_range(a..b + "</COMMENT>".len(), "");
    }
    let s = s.replace("\n<P>", "\n").replace("<P>", "\n");
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

impl InfoDisplay {
    /// Open (or close, if it's the one showing) the text for `tag`, read at `at`.
    pub fn toggle(&mut self, root: &std::path::Path, tag: &str, at: DVec3) -> Option<String> {
        if self.text.is_some() && self.tag.eq_ignore_ascii_case(tag) {
            self.text = None;
            return None;
        }
        if !self.tried {
            self.tried = true;
            self.pkg = unreal_pkg::Package::open(root.join("System").join("DeusExText.u")).ok();
        }
        // Lay D Denton's InformationDevices: her FemaleTextTag ("FemJC" + textTag) first, else the original.
        let fem = crate::travel::female().then(|| self.pkg.as_ref().and_then(|p| p.text_named(&format!("FemJC{tag}")))).flatten();
        let raw = fem.or_else(|| self.pkg.as_ref().and_then(|p| p.text_named(tag)));
        match raw {
            Some(r) => {
                self.text = Some(format_text(&r));
                self.tag = tag.to_string();
                self.at = at;
                None
            }
            None => Some(format!("(No text found for {tag})")),
        }
    }
}

pub fn spawn_info_panel(commands: &mut Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(8.0),
                left: Val::Percent(22.0),
                width: Val::Percent(56.0),
                padding: UiRect::all(Val::Px(14.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.82)),
            BorderColor::all(Color::srgba(0.6, 0.65, 0.7, 0.9)),
            Visibility::Hidden,
            Pickable::IGNORE,
            InfoPanel,
        ))
        .with_children(|c| {
            c.spawn((
                Text::new(""),
                TextFont {
                    font_size: bevy::text::FontSize::Px(16.0),
                    ..default()
                },
                TextColor(Color::srgb(0.9, 0.92, 0.95)),
                Pickable::IGNORE,
                InfoText,
            ));
        });
}

pub fn info_system(
    player: Res<Player>,
    mut info: ResMut<InfoDisplay>,
    mut panel: Single<&mut Visibility, With<InfoPanel>>,
    mut text: Single<&mut Text, With<InfoText>>,
) {
    let _prof = crate::prof::scope("info::info_system");
    // Walking away closes it.
    if info.text.is_some() && player.body.location.distance(info.at) > 160.0 {
        info.text = None;
    }
    match &info.text {
        Some(t) => {
            if text.0 != *t {
                text.0 = t.clone();
            }
            **panel = Visibility::Inherited;
        }
        None => **panel = Visibility::Hidden,
    }
}
