use std::path::PathBuf;

use loco_inertia::templates::{generate, verb_for};

/// A throwaway app skeleton with the files the generator injects into.
fn skeleton(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("loco-inertia-gen-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src/controllers")).unwrap();
    std::fs::write(root.join("src/controllers/mod.rs"), "pub mod home;\n").unwrap();
    std::fs::write(
        root.join("src/app.rs"),
        "fn routes() -> AppRoutes {\n        AppRoutes::with_default_routes()\n            .add_route(controllers::home::routes())\n}\n",
    )
    .unwrap();
    root
}

fn read(root: &std::path::Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

#[test]
fn verbs_follow_loco() {
    assert_eq!(verb_for("index"), "get");
    assert_eq!(verb_for("show"), "get");
    assert_eq!(verb_for("create"), "post");
    assert_eq!(verb_for("update"), "put");
    assert_eq!(verb_for("destroy"), "delete");
}

#[test]
fn generates_controller_pages_tests_and_injections() {
    let root = skeleton("full");
    let actions = ["index", "show", "create"].map(String::from);
    let messages = generate(&root, "my_app", "blog_posts", &actions).unwrap();
    assert_eq!(messages.len(), 4, "{messages:?}");

    let controller = read(&root, "src/controllers/blog_posts.rs");
    assert!(controller.contains("pub struct BlogPostsIndexProps"));
    assert!(controller.contains(r#"const COMPONENT: &'static str = "BlogPosts/Show";"#));
    assert!(controller.contains("#[ts(export)]"));
    // Non-GET actions redirect instead of rendering a page.
    assert!(!controller.contains("BlogPostsCreateProps"));
    assert!(controller.contains(".back()"));
    assert!(controller.contains(r#".prefix("blog_posts/")"#));
    assert!(controller.contains(r#".add("/", get(index))"#));
    assert!(controller.contains(r#".add("show", get(show))"#));
    assert!(controller.contains(r#".add("create", post(create))"#));

    let page = read(&root, "frontend/src/Pages/BlogPosts/Show.tsx");
    assert!(page
        .contains(r#"import type { BlogPostsShowProps } from "../../types/BlogPostsShowProps";"#));
    assert!(page.contains("export default function BlogPostsShow({ title }: BlogPostsShowProps)"));
    assert!(root.join("frontend/src/Pages/BlogPosts/Index.tsx").exists());
    assert!(!root
        .join("frontend/src/Pages/BlogPosts/Create.tsx")
        .exists());

    let test = read(&root, "tests/blog_posts_pages.rs");
    assert!(test.contains("use my_app::app::App;"));
    assert!(
        test.contains(r#".get("/blog_posts")"#) && test.contains(r#".get("/blog_posts/show")"#)
    );
    assert!(test.contains(r#""component":"BlogPosts/Show""#));
    assert!(!test.contains("renders_create"));

    assert!(read(&root, "src/controllers/mod.rs").contains("pub mod blog_posts;"));
    assert!(read(&root, "src/app.rs").contains(".add_route(controllers::blog_posts::routes())"));

    // Re-running keeps existing files and does not inject twice.
    generate(&root, "my_app", "blog_posts", &actions).unwrap();
    assert_eq!(
        read(&root, "src/controllers/mod.rs")
            .matches("blog_posts")
            .count(),
        1
    );

    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn defaults_to_an_index_page_and_requires_a_name() {
    let root = skeleton("default");
    generate(&root, "my_app", "posts", &[]).unwrap();
    assert!(root.join("frontend/src/Pages/Posts/Index.tsx").exists());
    assert!(generate(&root, "my_app", "", &[]).is_err());
    std::fs::remove_dir_all(&root).unwrap();
}
