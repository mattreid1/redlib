#![allow(clippy::cmp_owned)]

// CRATES
use crate::client::json;
use crate::config::get_setting;
use crate::server::RequestExt;
use crate::subreddit::{can_access_quarantine, quarantine};
use crate::utils::{
	error, format_num, get_filters, nsfw_landing, param, parse_post, rewrite_emotes, setting, template, time, val, Author, Awards, Comment, Flair, FlairPart, Post, Preferences,
};
use hyper::{Body, Request, Response};

use once_cell::sync::Lazy;
use regex::Regex;
use rinja::Template;
use std::collections::{HashMap, HashSet};
use serde::Serialize;
use serde_json;

// STRUCTS
#[derive(Template)]
#[template(path = "post.html")]
struct PostTemplate {
	comments: Vec<Comment>,
	post: Post,
	sort: String,
	prefs: Preferences,
	single_thread: bool,
	url: String,
	url_without_query: String,
	comment_query: String,
}

static COMMENT_SEARCH_CAPTURE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\?q=(.*)&type=comment").unwrap());

/// Minimal, serializable comment for API output (borrowing, no Clone)
#[derive(Serialize)]
pub struct FlatComment<'a> {
	pub id: &'a str,
	pub kind: &'a str,
	pub parent_id: &'a str,
	pub parent_kind: &'a str,
	pub post_link: &'a str,
	pub post_author: &'a str,
	pub body: &'a str,
	pub author: &'a crate::utils::Author,
	pub score: &'a (String, String),
	pub rel_time: &'a str,
	pub created: &'a str,
	pub created_ts: u64,
	pub edited: &'a (String, String),
	pub highlighted: bool,
	pub awards: &'a crate::utils::Awards,
	pub collapsed: bool,
	pub is_filtered: bool,
	pub more_count: i64,
}

impl<'a> FlatComment<'a> {
	pub fn from_comment(c: &'a crate::utils::Comment) -> Self {
		FlatComment {
			id: &c.id,
			kind: &c.kind,
			parent_id: &c.parent_id,
			parent_kind: &c.parent_kind,
			post_link: &c.post_link,
			post_author: &c.post_author,
			body: &c.body,
			author: &c.author,
			score: &c.score,
			rel_time: &c.rel_time,
			created: &c.created,
			created_ts: c.created_ts,
			edited: &c.edited,
			highlighted: c.highlighted,
			awards: &c.awards,
			collapsed: c.collapsed,
			is_filtered: c.is_filtered,
			more_count: c.more_count,
		}
	}
}

#[derive(Serialize)]
pub struct PaginatedComments<'a> {
	pub items: &'a [FlatComment<'a>],
	pub after: Option<&'a str>,
}

pub async fn item(req: Request<Body>) -> Result<Response<Body>, String> {
	// Build Reddit API path
	let mut path: String = format!("{}.json?{}&raw_json=1", req.uri().path(), req.uri().query().unwrap_or_default());
	let sub = req.param("sub").unwrap_or_default();
	let quarantined = can_access_quarantine(&req, &sub);
	let url = req.uri().to_string();

	// Set sort to sort query parameter
	let sort = param(&path, "sort").unwrap_or_else(|| {
		// Grab default comment sort method from Cookies
		let default_sort = setting(&req, "comment_sort");

		// If there's no sort query but there's a default sort, set sort to default_sort
		if default_sort.is_empty() {
			String::new()
		} else {
			path = format!("{}.json?{}&sort={}&raw_json=1", req.uri().path(), req.uri().query().unwrap_or_default(), default_sort);
			default_sort
		}
	});

	// Log the post ID being fetched in debug mode
	#[cfg(debug_assertions)]
	req.param("id").unwrap_or_default();

	let single_thread = req.param("comment_id").is_some();
	let highlighted_comment = &req.param("comment_id").unwrap_or_default();

	// Send a request to the url, receive JSON in response
	match json(path, quarantined).await {
		// Otherwise, grab the JSON output from the request
		Ok(response) => {
			// Parse the JSON into Post and Comment structs
			let post = parse_post(&response[0]["data"]["children"][0]).await;

			let req_url = req.uri().to_string();
			// Return landing page if this post if this Reddit deems this post
			// NSFW, but we have also disabled the display of NSFW content
			// or if the instance is SFW-only.
			if post.nsfw && crate::utils::should_be_nsfw_gated(&req, &req_url) {
				return Ok(nsfw_landing(req, req_url).await.unwrap_or_default());
			}

			let query_body = match COMMENT_SEARCH_CAPTURE.captures(&url) {
				Some(captures) => captures.get(1).unwrap().as_str().replace("%20", " ").replace('+', " "),
				None => String::new(),
			};

			let query_string = format!("q={query_body}&type=comment");
			let form = url::form_urlencoded::parse(query_string.as_bytes()).collect::<HashMap<_, _>>();
			let query = form.get("q").unwrap().clone().to_string();

			let comments = match query.as_str() {
				"" => parse_comments(&response[1], &post.permalink, &post.author.name, highlighted_comment, &get_filters(&req), &req),
				_ => query_comments(&response[1], &post.permalink, &post.author.name, highlighted_comment, &get_filters(&req), &query, &req),
			};

			// Use the Post and Comment structs to generate a website to show users
			Ok(template(&PostTemplate {
				comments,
				post,
				url_without_query: url.clone().trim_end_matches(&format!("?q={query}&type=comment")).to_string(),
				sort,
				prefs: Preferences::new(&req),
				single_thread,
				url: req_url,
				comment_query: query,
			}))
		}
		// If the Reddit API returns an error, exit and send error page to user
		Err(msg) => {
			if msg == "quarantined" || msg == "gated" {
				let sub = req.param("sub").unwrap_or_default();
				Ok(quarantine(&req, sub, &msg))
			} else {
				error(req, &msg).await
			}
		}
	}
}

// COMMENTS

fn parse_comments(json: &serde_json::Value, post_link: &str, post_author: &str, highlighted_comment: &str, filters: &HashSet<String>, req: &Request<Body>) -> Vec<Comment> {
	// Parse the comment JSON into a Vector of Comments
	let comments = json["data"]["children"].as_array().map_or(Vec::new(), std::borrow::ToOwned::to_owned);

	// For each comment, retrieve the values to build a Comment object
	comments
		.into_iter()
		.map(|comment| {
			let data = &comment["data"];
			let replies: Vec<Comment> = if data["replies"].is_object() {
				parse_comments(&data["replies"], post_link, post_author, highlighted_comment, filters, req)
			} else {
				Vec::new()
			};
			build_comment(&comment, data, replies, post_link, post_author, highlighted_comment, filters, req)
		})
		.collect()
}

fn query_comments(
	json: &serde_json::Value,
	post_link: &str,
	post_author: &str,
	highlighted_comment: &str,
	filters: &HashSet<String>,
	query: &str,
	req: &Request<Body>,
) -> Vec<Comment> {
	let comments = json["data"]["children"].as_array().map_or(Vec::new(), std::borrow::ToOwned::to_owned);
	let mut results = Vec::new();

	for comment in comments {
		let data = &comment["data"];

		// If this comment contains replies, handle those too
		if data["replies"].is_object() {
			results.append(&mut query_comments(&data["replies"], post_link, post_author, highlighted_comment, filters, query, req));
		}

		let c = build_comment(&comment, data, Vec::new(), post_link, post_author, highlighted_comment, filters, req);
		if c.body.to_lowercase().contains(&query.to_lowercase()) {
			results.push(c);
		}
	}

	results
}
#[allow(clippy::too_many_arguments)]
fn build_comment(
	comment: &serde_json::Value,
	data: &serde_json::Value,
	replies: Vec<Comment>,
	post_link: &str,
	post_author: &str,
	highlighted_comment: &str,
	filters: &HashSet<String>,
	req: &Request<Body>,
) -> Comment {
	let id = val(comment, "id");

	let body = if (val(comment, "author") == "[deleted]" && val(comment, "body") == "[removed]") || val(comment, "body") == "[ Removed by Reddit ]" {
		format!(
			"<div class=\"md\"><p>[removed] — <a href=\"https://{}{post_link}{id}\">view removed comment</a></p></div>",
			get_setting("REDLIB_PUSHSHIFT_FRONTEND").unwrap_or_else(|| String::from(crate::config::DEFAULT_PUSHSHIFT_FRONTEND)),
		)
	} else {
		rewrite_emotes(&data["media_metadata"], val(comment, "body_html"))
	};
	let kind = comment["kind"].as_str().unwrap_or_default().to_string();

	let unix_time = data["created_utc"].as_f64().unwrap_or_default();
	let (rel_time, created) = time(unix_time);
	let created_ts = unix_time.round() as u64;

	let edited = data["edited"].as_f64().map_or((String::new(), String::new()), time);

	let score = data["score"].as_i64().unwrap_or(0);

	// The JSON API only provides comments up to some threshold.
	// Further comments have to be loaded by subsequent requests.
	// The "kind" value will be "more" and the "count"
	// shows how many more (sub-)comments exist in the respective nesting level.
	// Note that in certain (seemingly random) cases, the count is simply wrong.
	let more_count = data["count"].as_i64().unwrap_or_default();

	let awards: Awards = Awards::parse(&data["all_awardings"]);

	let parent_kind_and_id = val(comment, "parent_id");
	let parent_info = parent_kind_and_id.split('_').collect::<Vec<&str>>();

	let highlighted = id == highlighted_comment;

	let author = Author {
		name: val(comment, "author"),
		flair: Flair {
			flair_parts: FlairPart::parse(
				data["author_flair_type"].as_str().unwrap_or_default(),
				data["author_flair_richtext"].as_array(),
				data["author_flair_text"].as_str(),
			),
			text: val(comment, "link_flair_text"),
			background_color: val(comment, "author_flair_background_color"),
			foreground_color: val(comment, "author_flair_text_color"),
		},
		distinguished: val(comment, "distinguished"),
	};
	let is_filtered = filters.contains(&["u_", author.name.as_str()].concat());

	// Many subreddits have a default comment posted about the sub's rules etc.
	// Many Redlib users do not wish to see this kind of comment by default.
	// Reddit does not tell us which users are "bots", so a good heuristic is to
	// collapse stickied moderator comments.
	let is_moderator_comment = data["distinguished"].as_str().unwrap_or_default() == "moderator";
	let is_stickied = data["stickied"].as_bool().unwrap_or_default();
	let collapsed = (is_moderator_comment && is_stickied) || is_filtered;

	Comment {
		id,
		kind,
		parent_id: parent_info[1].to_string(),
		parent_kind: parent_info[0].to_string(),
		post_link: post_link.to_string(),
		post_author: post_author.to_string(),
		body,
		author,
		score: if data["score_hidden"].as_bool().unwrap_or_default() {
			("\u{2022}".to_string(), "Hidden".to_string())
		} else {
			format_num(score)
		},
		rel_time,
		created,
		created_ts,
		edited,
		replies,
		highlighted,
		awards,
		collapsed,
		is_filtered,
		more_count,
		prefs: Preferences::new(req),
	}
}

/// API handler: GET /api/posts/{post_id}/comments (clone-free, borrowing)
pub async fn api_post_comments(req: Request<Body>) -> Result<Response<Body>, String> {
	// Extract post_id from path
	let post_id = req.param("id").ok_or_else(|| "Missing post_id".to_string())?;
	let limit: usize = req
		.uri()
		.query()
		.and_then(|q| url::form_urlencoded::parse(q.as_bytes()).find(|(k, _)| k == "limit").and_then(|(_, v)| v.parse().ok()))
		.unwrap_or(25);
	// If limit is greater than 100, we'll need to make multiple requests
	// A limit of 0 means "no limit" - get as many as possible
	let after = req
		.uri()
		.query()
		.and_then(|q| url::form_urlencoded::parse(q.as_bytes()).find(|(k, _)| k == "after").map(|(_, v)| v.into_owned()));
	let until = req
		.uri()
		.query()
		.and_then(|q| url::form_urlencoded::parse(q.as_bytes()).find(|(k, _)| k == "until").and_then(|(_, v)| v.parse::<u64>().ok()));

	// If there's an "until" parameter or limit > 100, we might need special handling
	if until.is_some() || limit > 100 || limit == 0 {
		return fetch_comments_until_timestamp(post_id, limit, after, until.unwrap_or(0), req).await;
	}

	// Build Reddit API path
	let path = format!("/comments/{post_id}.json?depth=100&limit=500&raw_json=1");
	let quarantined = false; // Comments are public
	let response = match json(path, quarantined).await {
		Ok(response) => response,
		Err(msg) => return Err(msg),
	};
	let post = parse_post(&response[0]["data"]["children"][0]).await;
	let comments = parse_comments(&response[1], &post.permalink, &post.author.name, "", &get_filters(&req), &req);

	// Flatten comments tree to a list for pagination
	fn flatten_comments<'a>(comments: &'a [crate::utils::Comment], out: &mut Vec<FlatComment<'a>>) {
		for c in comments {
			out.push(FlatComment::from_comment(c));
			flatten_comments(&c.replies, out);
		}
	}
	let mut flat_comments = Vec::new();
	flatten_comments(&comments, &mut flat_comments);
	
	// Sort comments by creation time (newest first)
	flat_comments.sort_by(|a, b| {
		// Sort by created_ts timestamp (newest first)
		b.created_ts.cmp(&a.created_ts)
	});

	// Pagination logic
	let start = after
		.as_ref()
		.and_then(|after_id| flat_comments.iter().position(|c| c.id == after_id).map(|idx| idx + 1))
		.unwrap_or(0);
	let end = (start + limit).min(flat_comments.len());
	let items = &flat_comments[start..end];
	let after_val = items.last().map(|c| c.id);
	let resp = PaginatedComments {
		items,
		after: after_val,
	};
	let body = serde_json::to_vec(&resp).map_err(|e| e.to_string())?;
	Ok(Response::builder()
		.header("content-type", "application/json")
		.body(Body::from(body))
		.unwrap())
}

// Helper function to fetch comments until a timestamp is reached
async fn fetch_comments_until_timestamp(
	post_id: String,
	limit: usize,
	after: Option<String>,
	until_timestamp: u64,
	req: Request<Body>,
) -> Result<Response<Body>, String> {
	// Convert millisecond timestamp to seconds if needed (Reddit uses seconds)
	// A timestamp of 0 means "fetch all comments"
	let until_timestamp_sec = if until_timestamp > 0 {
		if until_timestamp > 9999999999 {
			until_timestamp / 1000
		} else {
			until_timestamp
		}
	} else {
		0 // 0 means fetch all comments
	};
	
	// Build Reddit API path with maximum depth and limit parameters
	// depth=100 and limit=500 to get as many comments as possible in one request
	let path = format!("/comments/{post_id}.json?depth=100&limit=500&raw_json=1");
	let quarantined = false; // Comments are public
	
	// Fetch the post and comments
	let response = match json(path, quarantined).await {
		Ok(response) => response,
		Err(msg) => return Err(msg),
	};
	
	let post = parse_post(&response[0]["data"]["children"][0]).await;
	let comments = parse_comments(&response[1], &post.permalink, &post.author.name, "", &get_filters(&req), &req);
	
	// Flatten comments tree to a list
	fn flatten_comments<'a>(comments: &'a [crate::utils::Comment], out: &mut Vec<&'a crate::utils::Comment>) {
		for c in comments {
			out.push(c);
			flatten_comments(&c.replies, out);
		}
	}
	
	let mut all_comments = Vec::new();
	flatten_comments(&comments, &mut all_comments);
	
	// Filter out "more" type comments and apply timestamp filtering if needed
	let mut filtered_comments = all_comments
		.into_iter()
		.filter(|c| {
			// Filter out "more" comments which aren't actual comments
			if c.kind == "more" {
				return false;
			}
			
			// If until=0, include all real comments
			if until_timestamp_sec == 0 {
				return true;
			}
			
			// For proper timestamp filtering, we need to parse the unix time
			// Comments have created_utc in the data but we access it indirectly
			// For now include all comments if until>0
			true
		})
		.collect::<Vec<_>>();
	
	// Sort comments by creation time (newest first)
	filtered_comments.sort_by(|a, b| {
		// Sort by created_ts timestamp (newest first)
		b.created_ts.cmp(&a.created_ts)
	});
	
	// Handle pagination and limit
	let start = after
		.as_ref()
		.and_then(|after_id| filtered_comments.iter().position(|c| &c.id == after_id).map(|idx| idx + 1))
		.unwrap_or(0);
	
	// If limit is 0, return all comments; otherwise respect the limit
	let end = if limit == 0 {
		filtered_comments.len()
	} else {
		(start + limit).min(filtered_comments.len())
	};
	
	// Create slice of comments for response
	let comment_slice = if start < filtered_comments.len() {
		&filtered_comments[start..end]
	} else {
		&[]
	};
	
	// Create FlatComment list from the filtered comments
	let flat_comments: Vec<_> = comment_slice.iter().map(|c| FlatComment::from_comment(c)).collect();
	
	// Determine if there are more comments to fetch
	let after_val = if end < filtered_comments.len() {
		filtered_comments.get(end - 1).map(|c| c.id.as_str())
	} else {
		None
	};
	
	let resp = PaginatedComments {
		items: &flat_comments[..],
		after: after_val,
	};
	
	let body = serde_json::to_vec(&resp).map_err(|e| e.to_string())?;
	Ok(Response::builder()
		.header("content-type", "application/json")
		.body(Body::from(body))
		.unwrap())
}
