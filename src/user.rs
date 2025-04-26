#![allow(clippy::cmp_owned)]

// CRATES
use crate::client::json;
use crate::server::RequestExt;
use crate::utils::{error, filter_posts, format_url, get_filters, nsfw_landing, param, setting, template, Post, Preferences, User};
use crate::{config, utils};
use chrono::DateTime;
use htmlescape::decode_html;
use hyper::{Body, Request, Response};
use rinja::Template;
use time::{macros::format_description, OffsetDateTime};
use serde::Serialize;

// STRUCTS
#[derive(Template)]
#[template(path = "user.html")]
struct UserTemplate {
	user: User,
	posts: Vec<Post>,
	sort: (String, String),
	ends: (String, String),
	/// "overview", "comments", or "submitted"
	listing: String,
	prefs: Preferences,
	url: String,
	redirect_url: String,
	/// Whether the user themself is filtered.
	is_filtered: bool,
	/// Whether all fetched posts are filtered (to differentiate between no posts fetched in the first place,
	/// and all fetched posts being filtered).
	all_posts_filtered: bool,
	/// Whether all posts were hidden because they are NSFW (and user has disabled show NSFW)
	all_posts_hidden_nsfw: bool,
	no_posts: bool,
}

/// Minimal, serializable user post for API output (borrowing, no Clone)
#[derive(Serialize)]
pub struct FlatUserPost<'a> {
	pub id: &'a str,
	pub title: &'a str,
	pub community: &'a str,
	pub body: &'a str,
	pub author: &'a crate::utils::Author,
	pub permalink: &'a str,
	pub score: &'a (String, String),
	pub upvote_ratio: i64,
	pub post_type: &'a str,
	pub flair: &'a crate::utils::Flair,
	pub flags: &'a crate::utils::Flags,
	pub rel_time: &'a str,
	pub created: &'a str,
	pub created_ts: u64,
	pub num_duplicates: u64,
	pub comments: &'a (String, String),
	pub awards: &'a crate::utils::Awards,
	pub nsfw: bool,
}

impl<'a> FlatUserPost<'a> {
	pub fn from_post(p: &'a crate::utils::Post) -> Self {
		FlatUserPost {
			id: &p.id,
			title: &p.title,
			community: &p.community,
			body: &p.body,
			author: &p.author,
			permalink: &p.permalink,
			score: &p.score,
			upvote_ratio: p.upvote_ratio,
			post_type: &p.post_type,
			flair: &p.flair,
			flags: &p.flags,
			rel_time: &p.rel_time,
			created: &p.created,
			created_ts: p.created_ts,
			num_duplicates: p.num_duplicates,
			comments: &p.comments,
			awards: &p.awards,
			nsfw: p.nsfw,
		}
	}
}

#[derive(Serialize)]
pub struct PaginatedUserPosts<'a> {
	pub items: &'a [FlatUserPost<'a>],
	pub after: Option<&'a str>,
}

// FUNCTIONS
pub async fn profile(req: Request<Body>) -> Result<Response<Body>, String> {
	let listing = req.param("listing").unwrap_or_else(|| "overview".to_string());

	// Build the Reddit JSON API path
	let path = format!(
		"/user/{}/{listing}.json?{}&raw_json=1",
		req.param("name").unwrap_or_else(|| "reddit".to_string()),
		req.uri().query().unwrap_or_default(),
	);
	let url = String::from(req.uri().path_and_query().map_or("", |val| val.as_str()));
	let redirect_url = url[1..].replace('?', "%3F").replace('&', "%26");

	// Retrieve other variables from Redlib request
	let sort = param(&path, "sort").unwrap_or_default();
	let username = req.param("name").unwrap_or_default();

	// Retrieve info from user about page.
	let user = user(&username).await.unwrap_or_default();

	let req_url = req.uri().to_string();
	// Return landing page if this post if this Reddit deems this user NSFW,
	// but we have also disabled the display of NSFW content or if the instance
	// is SFW-only.
	if user.nsfw && crate::utils::should_be_nsfw_gated(&req, &req_url) {
		return Ok(nsfw_landing(req, req_url).await.unwrap_or_default());
	}

	let filters = get_filters(&req);
	if filters.contains(&["u_", &username].concat()) {
		Ok(template(&UserTemplate {
			user,
			posts: Vec::new(),
			sort: (sort, param(&path, "t").unwrap_or_default()),
			ends: (param(&path, "after").unwrap_or_default(), String::new()),
			listing,
			prefs: Preferences::new(&req),
			url,
			redirect_url,
			is_filtered: true,
			all_posts_filtered: false,
			all_posts_hidden_nsfw: false,
			no_posts: false,
		}))
	} else {
		// Request user posts/comments from Reddit
		match Post::fetch(&path, false).await {
			Ok((mut posts, after)) => {
				let (_, all_posts_filtered) = filter_posts(&mut posts, &filters);
				let no_posts = posts.is_empty();
				let all_posts_hidden_nsfw = !no_posts && (posts.iter().all(|p| p.flags.nsfw) && setting(&req, "show_nsfw") != "on");
				Ok(template(&UserTemplate {
					user,
					posts,
					sort: (sort, param(&path, "t").unwrap_or_default()),
					ends: (param(&path, "after").unwrap_or_default(), after),
					listing,
					prefs: Preferences::new(&req),
					url,
					redirect_url,
					is_filtered: false,
					all_posts_filtered,
					all_posts_hidden_nsfw,
					no_posts,
				}))
			}
			// If there is an error show error page
			Err(msg) => error(req, &msg).await,
		}
	}
}

// USER
async fn user(name: &str) -> Result<User, String> {
	// Build the Reddit JSON API path
	let path: String = format!("/user/{name}/about.json?raw_json=1");

	// Send a request to the url
	json(path, false).await.map(|res| {
		// Grab creation date as unix timestamp
		let created_unix = res["data"]["created"].as_f64().unwrap_or(0.0).round() as i64;
		let created = OffsetDateTime::from_unix_timestamp(created_unix).unwrap_or(OffsetDateTime::UNIX_EPOCH);

		// Closure used to parse JSON from Reddit APIs
		let about = |item| res["data"]["subreddit"][item].as_str().unwrap_or_default().to_string();

		// Parse the JSON output into a User struct
		User {
			name: res["data"]["name"].as_str().unwrap_or(name).to_owned(),
			title: about("title"),
			icon: format_url(&about("icon_img")),
			karma: res["data"]["total_karma"].as_i64().unwrap_or(0),
			created: created.format(format_description!("[month repr:short] [day] '[year repr:last_two]")).unwrap_or_default(),
			banner: about("banner_img"),
			description: about("public_description"),
			nsfw: res["data"]["subreddit"]["over_18"].as_bool().unwrap_or_default(),
		}
	})
}

pub async fn rss(req: Request<Body>) -> Result<Response<Body>, String> {
	if config::get_setting("REDLIB_ENABLE_RSS").is_none() {
		return Ok(error(req, "RSS is disabled on this instance.").await.unwrap_or_default());
	}
	use crate::utils::rewrite_urls;
	use hyper::header::CONTENT_TYPE;
	use rss::{ChannelBuilder, Item};

	// Get user
	let user_str = req.param("name").unwrap_or_default();

	let listing = req.param("listing").unwrap_or_else(|| "overview".to_string());

	// Get path
	let path = format!("/user/{user_str}/{listing}.json?{}&raw_json=1", req.uri().query().unwrap_or_default(),);

	// Get user
	let user_obj = user(&user_str).await.unwrap_or_default();

	// Get posts
	let (posts, _) = Post::fetch(&path, false).await?;

	// Build the RSS feed
	let channel = ChannelBuilder::default()
		.title(user_str)
		.description(user_obj.description)
		.items(
			posts
				.into_iter()
				.map(|post| Item {
					title: Some(post.title.to_string()),
					link: Some(format_url(&utils::get_post_url(&post))),
					author: Some(post.author.name),
					pub_date: Some(DateTime::from_timestamp(post.created_ts as i64, 0).unwrap_or_default().to_rfc2822()),
					content: Some(rewrite_urls(&decode_html(&post.body).unwrap())),
					..Default::default()
				})
				.collect::<Vec<_>>(),
		)
		.build();

	// Serialize the feed to RSS
	let body = channel.to_string().into_bytes();

	// Create the HTTP response
	let mut res = Response::new(Body::from(body));
	res.headers_mut().insert(CONTENT_TYPE, hyper::header::HeaderValue::from_static("application/rss+xml"));

	Ok(res)
}

/// API handler: GET /api/users/:user/posts (clone-free, borrowing)
pub async fn api_user_posts(req: Request<Body>) -> Result<Response<Body>, String> {
	let user = req.param("user").ok_or_else(|| "Missing user".to_string())?;
	let listing = req
		.uri()
		.query()
		.and_then(|q| url::form_urlencoded::parse(q.as_bytes()).find(|(k, _)| k == "where").map(|(_, v)| v.into_owned()))
		.unwrap_or_else(|| "overview".to_string());
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
	let sort = req
		.uri()
		.query()
		.and_then(|q| url::form_urlencoded::parse(q.as_bytes()).find(|(k, _)| k == "sort").map(|(_, v)| v.into_owned()));
	let until = req
		.uri()
		.query()
		.and_then(|q| url::form_urlencoded::parse(q.as_bytes()).find(|(k, _)| k == "until").and_then(|(_, v)| v.parse::<u64>().ok()));

	// If there's an "until" parameter or limit > 100, we might need to make multiple requests
	if until.is_some() || limit > 100 || limit == 0 {
		return fetch_posts_until_timestamp(user, listing, limit, after, sort, until.unwrap_or(0)).await;
	}

	// Build Reddit API path, including after, limit, and sort if present
	let mut path = format!("/user/{}/{}.json?raw_json=1", user, listing);
	let mut params = vec![];
	if let Some(ref after_val) = after {
		params.push(format!("after={}", after_val));
	}
	if limit != 25 {
		params.push(format!("limit={}", limit));
	}
	if let Some(ref sort_val) = sort {
		params.push(format!("sort={}", sort_val));
	}
	if !params.is_empty() {
		path.push('&');
		path.push_str(&params.join("&"));
	}
	let quarantined = false;
	let (posts, reddit_after) = match crate::utils::Post::fetch(&path, quarantined).await {
		Ok((posts, after)) => (posts, after),
		Err(msg) => return Err(msg),
	};

	// Build borrowed FlatUserPost list
	let flat_posts: Vec<_> = posts.iter().map(FlatUserPost::from_post).collect();

	// No local pagination; just return what Reddit gave us
	let items = &flat_posts[..];
	let after_val = if reddit_after.is_empty() { None } else { Some(reddit_after.as_str()) };
	let resp = PaginatedUserPosts {
		items,
		after: after_val,
	};
	let body = serde_json::to_vec(&resp).map_err(|e| e.to_string())?;
	Ok(Response::builder()
		.header("content-type", "application/json")
		.body(Body::from(body))
		.unwrap())
}

// Helper function to fetch posts until a timestamp is reached
async fn fetch_posts_until_timestamp(
	user: String,
	listing: String,
	limit: usize,
	after: Option<String>,
	sort: Option<String>,
	until_timestamp: u64,
) -> Result<Response<Body>, String> {
	// Convert millisecond timestamp to seconds if needed (Reddit uses seconds)
	// A timestamp of 0 means "fetch all posts"
	let until_timestamp_sec = if until_timestamp > 0 {
		if until_timestamp > 9999999999 {
			until_timestamp / 1000
		} else {
			until_timestamp
		}
	} else {
		0 // 0 means fetch all posts
	};
	
	let mut all_posts = Vec::new();
	let mut current_after = after;
	let max_requests = 25; // Increase limit for larger fetches
	let quarantined = false;
	let request_limit = 100; // Reddit API max limit per request
	
	// Fetch posts in batches until we reach the timestamp or run out of posts
	for _ in 0..max_requests {
		let mut path = format!("/user/{}/{}.json?raw_json=1", user, listing);
		let mut params = vec![];
		
		if let Some(ref after_val) = current_after {
			params.push(format!("after={}", after_val));
		}
		
		// Always use maximum limit for Reddit API
		params.push(format!("limit={}", request_limit));
		
		if let Some(ref sort_val) = sort {
			params.push(format!("sort={}", sort_val));
		}
		
		if !params.is_empty() {
			path.push('&');
			path.push_str(&params.join("&"));
		}
		
		// Fetch posts
		let (batch_posts, reddit_after) = match crate::utils::Post::fetch(&path, quarantined).await {
			Ok((posts, after)) => (posts, after),
			Err(msg) => return Err(msg),
		};
		
		if batch_posts.is_empty() {
			break;
		}
		
		// Check if we've reached the timestamp
		let oldest_post_time = batch_posts.iter().map(|p| p.created_ts).min().unwrap_or(0);
		
		// Add posts to our collection if they're newer than the until timestamp or if until=0
		if until_timestamp_sec == 0 {
			// If until=0, add all posts
			all_posts.extend(batch_posts);
		} else {
			// Otherwise filter by timestamp
			all_posts.extend(batch_posts.into_iter().filter(|p| p.created_ts >= until_timestamp_sec));
		}
		
		// If we've reached the user's requested limit, stop fetching more
		if limit != 0 && all_posts.len() >= limit {
			break;
		}
		
		// If the oldest post is older than our until timestamp or there are no more posts, stop
		if until_timestamp_sec > 0 && oldest_post_time < until_timestamp_sec {
			break;
		}
		
		// If there are no more posts to fetch, stop
		if reddit_after.is_empty() {
			break;
		}
		
		// Update for next iteration
		current_after = Some(reddit_after);
	}
	
	// Limit results to original requested limit if not 0
	if limit != 0 && all_posts.len() > limit {
		all_posts.truncate(limit);
	}
	
	// Create flat posts from collected posts
	let flat_posts: Vec<_> = all_posts.iter().map(FlatUserPost::from_post).collect();
	
	// Determine if there are more posts to fetch
	let after_val = if all_posts.is_empty() {
		None
	} else if current_after.is_some() && current_after.as_deref() != Some("") {
		// If we still had more posts to fetch
		all_posts.last().map(|p| p.id.as_str())
	} else {
		None
	};
	
	let resp = PaginatedUserPosts {
		items: &flat_posts[..],
		after: after_val,
	};
	
	let body = serde_json::to_vec(&resp).map_err(|e| e.to_string())?;
	Ok(Response::builder()
		.header("content-type", "application/json")
		.body(Body::from(body))
		.unwrap())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fetching_user() {
	let user = user("spez").await;
	assert!(user.is_ok());
	assert!(user.unwrap().karma > 100);
}
