use crate::document::{
    LinkSelectionError, MAXIMUM_REMOTE_IMAGES, UrlRejection, remote_images, resolve_url,
    select_link,
};
use crate::{CssSelector, LinkSelector};
use url::Url;

/**
 * Build a CSS link selector.
 */
fn css(source: &str) -> LinkSelector {
    LinkSelector::Css(CssSelector::parse(source).unwrap())
}

#[test]
/**
 * Verify the documented URL resolution and rejection policy.
 */
fn url_resolution_policy() {
    let base = Url::parse("https://app.test/mail/").unwrap();
    assert_eq!(
        resolve_url(None, " https://A.test/x#frag ").unwrap(),
        "https://a.test/x"
    );
    assert_eq!(
        resolve_url(None, "//cdn.test/p.gif").unwrap(),
        "https://cdn.test/p.gif"
    );
    assert_eq!(
        resolve_url(Some(&base), "verify?id=1").unwrap(),
        "https://app.test/mail/verify?id=1"
    );
    assert_eq!(
        resolve_url(None, "verify?id=1"),
        Err(UrlRejection::Relative)
    );
    assert_eq!(resolve_url(None, ""), Err(UrlRejection::Empty));
    for (reference, scheme) in [
        ("mailto:a@b.test", "mailto"),
        ("javascript:alert(1)", "javascript"),
        ("JaVaScRiPt:alert(1)", "javascript"),
        ("data:text/html,x", "data"),
        ("file:///etc/passwd", "file"),
        ("ftp://host.test/x", "ftp"),
    ] {
        assert_eq!(
            resolve_url(Some(&base), reference),
            Err(UrlRejection::UnsupportedScheme(scheme.into()))
        );
    }
    assert_eq!(
        resolve_url(None, "https://user:secret@host.test/"),
        Err(UrlRejection::EmbeddedCredentials)
    );
    assert_eq!(
        resolve_url(None, "https://user@host.test/"),
        Err(UrlRejection::EmbeddedCredentials)
    );
    assert_eq!(resolve_url(None, "http://[::1"), Err(UrlRejection::Invalid));
    let long = format!("https://host.test/{}", "a".repeat(3000));
    assert_eq!(resolve_url(None, &long), Err(UrlRejection::TooLong));
}

#[test]
/**
 * Verify remote images are discovered the way a mail client would load them.
 */
fn remote_images_cover_tracking_pixels_and_backgrounds() {
    let html = r#"
        <base href="https://base.test/news/">
        <table background="bg.png"><tr><td background="https://cdn.test/cell.png">
        <img src="https://track.test/open.gif?u=1" width="1" height="1">
        <img src="https://track.test/open.gif?u=1">
        <img srcset="https://cdn.test/a.png 1x, https://cdn.test/a@2x.png 2x">
        <img src="cid:logo@x"><img src="data:image/png;base64,AAAA">
        <img src="javascript:alert(1)"><img src="">
        <input type="IMAGE" src="https://cdn.test/button.png"><input type="text" src="https://cdn.test/no.png">
        <script>document.write('<img src="https://evil.test/x">')</script>
    "#;
    let images = remote_images(html);
    assert_eq!(
        images.urls,
        [
            "https://base.test/news/bg.png",
            "https://cdn.test/cell.png",
            "https://track.test/open.gif?u=1",
            "https://cdn.test/a.png",
            "https://cdn.test/button.png",
        ]
    );
    assert_eq!(images.embedded, 2);
    assert_eq!(images.rejected, 1);
    assert_eq!(images.omitted, 0);
}

#[test]
/**
 * Verify relative images without a base are rejected and the image limit is enforced.
 */
fn remote_image_limits_and_relative_references() {
    let mut html = String::from("<img src='/relative.png'>");
    for index in 0..40 {
        html.push_str(&format!("<img src='https://cdn.test/{index}.png'>"));
    }
    let images = remote_images(&html);
    assert_eq!(images.urls.len(), MAXIMUM_REMOTE_IMAGES);
    assert_eq!(images.omitted, 40 - MAXIMUM_REMOTE_IMAGES);
    assert_eq!(images.rejected, 1);
    assert_eq!(remote_images("   "), Default::default());
}

#[test]
/**
 * Verify CSS selection takes the first matching link, or the link enclosing a matched element.
 */
fn css_selection() {
    let html = r#"<p class="verify">not a link</p>
        <a class="verify" href="https://app.test/verify?t=1">Verify</a>
        <a class="verify" href="https://app.test/verify?t=2">Again</a>
        <a href="https://app.test/cta"><span class="cta">Go <b>now</b></span></a>"#;
    let link = select_link(html, &css("a.verify")).unwrap();
    assert_eq!(link.url, "https://app.test/verify?t=1");
    assert_eq!(link.text, "Verify");
    let link = select_link(html, &css(".verify")).unwrap();
    assert_eq!(
        link.url, "https://app.test/verify?t=1",
        "the non-link paragraph is skipped"
    );
    let link = select_link(html, &css("span.cta")).unwrap();
    assert_eq!(
        (link.url.as_str(), link.text.as_str()),
        ("https://app.test/cta", "Go now")
    );
    assert_eq!(
        select_link(html, &css("p")),
        Err(LinkSelectionError::NotALink(1))
    );
    assert_eq!(
        select_link(html, &css("a.missing")),
        Err(LinkSelectionError::NoCssMatch)
    );
}

#[test]
/**
 * Verify text selection collapses whitespace, ignores ASCII case, supports globs and alt text.
 */
fn text_selection() {
    let html = r#"<a href="https://app.test/1">Read   more</a>
        <a href="https://app.test/2">
            Confirm   your
            account
        </a>
        <a href="https://app.test/3"><img src="x.png" alt="Unsubscribe"></a>
        <map><area href="https://app.test/4" alt="Region"></map>"#;
    let select = |text: &str| select_link(html, &LinkSelector::Text(text.into()));
    assert_eq!(
        select("confirm YOUR account").unwrap().url,
        "https://app.test/2"
    );
    assert_eq!(select("Confirm*").unwrap().url, "https://app.test/2");
    assert_eq!(select("unsubscribe").unwrap().url, "https://app.test/3");
    assert_eq!(select("Region").unwrap().url, "https://app.test/4");
    assert_eq!(select("Confirm"), Err(LinkSelectionError::NoTextMatch(4)));
}

#[test]
/**
 * Verify URL pattern selection matches resolved URLs with case-sensitive globs.
 */
fn href_selection() {
    let html = r#"<base href="https://app.test/">
        <a href="settings">Settings</a>
        <a href="/lists/unsubscribe?id=7">Leave</a>"#;
    let select = |pattern: &str| select_link(html, &LinkSelector::Href(pattern.into()));
    assert_eq!(
        select("*unsubscribe*").unwrap().url,
        "https://app.test/lists/unsubscribe?id=7"
    );
    assert_eq!(
        select("https://app.test/settings").unwrap().text,
        "Settings"
    );
    assert_eq!(
        select("*UNSUBSCRIBE*"),
        Err(LinkSelectionError::NoHrefMatch(2))
    );
}

#[test]
/**
 * Verify selected links with dangerous or unresolvable URLs are refused rather than skipped.
 */
fn unsafe_selected_links_are_refused() {
    let html = r#"<a id="mail" href="mailto:help@app.test">Email us</a>
        <a id="script" href=" javascript:alert(1)">Run</a>
        <a id="relative" href="/verify">Verify</a>
        <a id="secret" href="https://user:pw@app.test/">Login</a>"#;
    let reason = |selector: &str| match select_link(html, &css(selector)) {
        Err(LinkSelectionError::Unfollowable { reason, .. }) => reason,
        other => panic!("expected an unfollowable link, got {other:?}"),
    };
    assert_eq!(
        reason("#mail"),
        UrlRejection::UnsupportedScheme("mailto".into())
    );
    assert_eq!(
        reason("#script"),
        UrlRejection::UnsupportedScheme("javascript".into())
    );
    assert_eq!(reason("#relative"), UrlRejection::Relative);
    assert_eq!(reason("#secret"), UrlRejection::EmbeddedCredentials);
}

#[test]
/**
 * Verify malformed HTML is repaired rather than rejected, and missing HTML is reported.
 */
fn malformed_html_is_handled() {
    let html =
        "<div><table><tr><td><a class=go href='https://app.test/go'>Go<td></table><p><b>unclosed";
    assert_eq!(
        select_link(html, &css("a.go")).unwrap().url,
        "https://app.test/go"
    );
    assert_eq!(select_link("", &css("a")), Err(LinkSelectionError::NoHtml));
    assert_eq!(
        select_link("<a href='x'", &css("a")),
        Err(LinkSelectionError::NoCssMatch),
        "an unterminated tag produces no element"
    );
    assert_eq!(
        remote_images("<p><img src='https://cdn.test/a.png'><td><b>unclosed").urls,
        ["https://cdn.test/a.png"]
    );
}
