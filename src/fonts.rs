//! インストール済みフォントの一覧（Windows の DirectWrite から取得）
//!
//! ユーザー単位でインストールしたフォントも含む。表示用の日本語名と、CSS で指定する英語名の両方を返す。

use serde::Serialize;
use windows::core::{w, BOOL};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection, IDWriteLocalizedStrings, DWRITE_FACTORY_TYPE_SHARED,
};

#[derive(Clone, Debug, Serialize)]
pub struct FontInfo {
    /// CSS の font-family に使う名前（英語名。無ければ最初の名前）
    pub family: String,
    /// 画面に出す名前（日本語名があればそれ）
    pub label: String,
}

fn localized(names: &IDWriteLocalizedStrings, locale: windows::core::PCWSTR) -> Option<String> {
    unsafe {
        let mut index = 0u32;
        let mut exists = BOOL(0);
        names.FindLocaleName(locale, &mut index, &mut exists).ok()?;
        if !exists.as_bool() {
            return None;
        }
        get_string(names, index)
    }
}

fn get_string(names: &IDWriteLocalizedStrings, index: u32) -> Option<String> {
    unsafe {
        let len = names.GetStringLength(index).ok()? as usize;
        let mut buf = vec![0u16; len + 1];
        names.GetString(index, &mut buf).ok()?;
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}

pub fn list() -> Result<Vec<FontInfo>, String> {
    unsafe {
        let factory: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).map_err(|e| e.to_string())?;
        let mut coll: Option<IDWriteFontCollection> = None;
        factory.GetSystemFontCollection(&mut coll, false).map_err(|e| e.to_string())?;
        let coll = coll.ok_or("フォント一覧を取得できません")?;
        let mut out = vec![];
        for i in 0..coll.GetFontFamilyCount() {
            let Ok(family) = coll.GetFontFamily(i) else { continue };
            let Ok(names) = family.GetFamilyNames() else { continue };
            let en = localized(&names, w!("en-us")).or_else(|| get_string(&names, 0));
            let Some(en) = en else { continue };
            let ja = localized(&names, w!("ja-jp"));
            let label = match ja {
                Some(ja) if ja != en => format!("{ja}（{en}）"),
                _ => en.clone(),
            };
            out.push(FontInfo { family: en, label });
        }
        out.sort_by(|a, b| a.label.cmp(&b.label));
        out.dedup_by(|a, b| a.family == b.family);
        Ok(out)
    }
}
