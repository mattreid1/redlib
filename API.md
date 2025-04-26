# Redlib API Documentation

This document describes the JSON API endpoints for Redlib. All endpoints return data in JSON format and support pagination where applicable.

---

## 1. Get Comments for a Post

**Endpoint:**
```
GET /api/post/{post_id}/comments
```

**Query Parameters:**
- `limit` (optional, integer): Maximum number of comments to return (default: 25, use 0 for no limit)
- `after` (optional, string): Return comments after this comment ID (for pagination)
- `until` (optional, integer): Unix timestamp in milliseconds - fetch comments until this time point (use 0 to fetch all comments)

**Example Request:**
```
GET /api/post/abc123/comments?limit=10&after=def456
```

**Example Request with timestamp:**
```
GET /api/post/abc123/comments?until=1625097600000
```

**Example Response:**
```json
{
  "items": [
    { "id": "ghi789", "author": "user1", "body": "Comment text", ... },
    ...
  ],
  "after": "ghi789"
}
```

**Note:** Comments are returned sorted by creation time, with the newest comments first.

---

## 2. Get Posts in a Subreddit

**Endpoint:**
```
GET /api/r/{subreddit}/posts
```

**Query Parameters:**
- `limit` (optional, integer): Maximum number of posts to return (default: 25, use 0 for no limit)
- `after` (optional, string): Return posts after this post ID (for pagination)
- `sort` (optional, string): Sort order (`hot`, `new`, `top`, etc.; default: `hot`)
- `until` (optional, integer): Unix timestamp in milliseconds - fetch posts until this time point (use 0 to fetch all posts)

**Example Request:**
```
GET /api/r/rust/posts?limit=5&after=xyz123&sort=new
```

**Example Request with timestamp:**
```
GET /api/r/rust/posts?until=1625097600000&limit=1000
```

**Example Response:**
```json
{
  "items": [
    { "id": "abc123", "title": "Post title", "author": "user2", ... },
    ...
  ],
  "after": "abc123"
}
```

---

## 3. Get Posts for a User

**Endpoint:**
```
GET /api/user/{user_id}/posts
```

**Query Parameters:**
- `where` (optional, string): Which listing to return (`overview`, `submitted`, `comments`; default: `overview`)
- `limit` (optional, integer): Maximum number of posts to return (default: 25, use 0 for no limit)
- `after` (optional, string): Return posts after this post ID (for pagination)
- `sort` (optional, string): Sort order (`new`, `top`, etc.; default: `new`)
- `until` (optional, integer): Unix timestamp in milliseconds - fetch posts until this time point (use 0 to fetch all posts)

**Example Request:**
```
GET /api/user/spez/posts?where=overview&limit=10&sort=top
```

**Example Request with timestamp:**
```
GET /api/user/Evening-Basil7333/posts?until=0&limit=1000000
```

**Example Response:**
```json
{
  "items": [
    { "id": "def456", "title": "Another post", "author": "spez", ... },
    ...
  ],
  "after": "def456"
}
```

---

## Notes
- All endpoints return a JSON object with an `items` array and an `after` string for pagination.
- If there are no more items, `after` will be `null` or omitted.
- The `until` parameter accepts a Unix timestamp in milliseconds and fetches posts/comments up to that point in time.
- Setting `until=0` will fetch all available posts/comments (up to the limit).
- Setting `limit=0` removes the item count restriction (up to implementation limits).
- All responses use camelCase for field names.
- Error responses will use standard HTTP status codes and a JSON body: `{ "error": "message" }` 