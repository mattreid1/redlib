# Redlib API Documentation

This document describes the JSON API endpoints for Redlib. All endpoints return data in JSON format and support pagination where applicable.

---

## 1. Get Comments for a Post

**Endpoint:**
```
GET /api/post/{post_id}/comments
```

**Query Parameters:**
- `limit` (optional, integer): Maximum number of comments to return (default: 25, max: 100)
- `after` (optional, string): Return comments after this comment ID (for pagination)

**Example Request:**
```
GET /api/post/abc123/comments?limit=10&after=def456
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

---

## 2. Get Posts in a Subreddit

**Endpoint:**
```
GET /api/r/{subreddit}/posts
```

**Query Parameters:**
- `limit` (optional, integer): Maximum number of posts to return (default: 25, max: 100)
- `after` (optional, string): Return posts after this post ID (for pagination)
- `sort` (optional, string): Sort order (`hot`, `new`, `top`, etc.; default: `hot`)

**Example Request:**
```
GET /api/r/rust/posts?limit=5&after=xyz123&sort=new
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
- `limit` (optional, integer): Maximum number of posts to return (default: 25, max: 100)
- `after` (optional, string): Return posts after this post ID (for pagination)
- `sort` (optional, string): Sort order (`new`, `top`, etc.; default: `new`)

**Example Request:**
```
GET /api/user/spez/posts?where=overview&limit=10&sort=top
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
- All responses use camelCase for field names.
- Error responses will use standard HTTP status codes and a JSON body: `{ "error": "message" }` 