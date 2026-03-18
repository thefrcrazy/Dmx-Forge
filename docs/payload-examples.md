# Payload Examples

Ce document fournit des exemples representatifs pour tester DmxForge ou comprendre les champs importants par provider.

Les exemples sont volontairement minimaux. Les providers reels envoient souvent beaucoup plus de donnees.

## URL webhook

Format :

```text
POST /webhooks/{provider}/{token}
```

Exemples :

- `POST /webhooks/github/<token>`
- `POST /webhooks/gitlab/<token>`
- `POST /webhooks/gitea/<token>`

## GitHub

### Headers attendus

| Header | Usage |
| --- | --- |
| `X-GitHub-Event` | Type d'evenement |
| `X-Hub-Signature-256` | Verification HMAC SHA-256 si un secret est configure |
| `X-Hub-Signature` | Fallback HMAC SHA-1 si necessaire |

### Exemple `push`

```http
POST /webhooks/github/abc123
Content-Type: application/json
X-GitHub-Event: push
X-Hub-Signature-256: sha256=<signature>
```

```json
{
  "ref": "refs/heads/main",
  "compare": "https://github.com/acme/dmxforge/compare/abc1234...def5678",
  "head_commit": {
    "message": "Ship dashboard shell",
    "timestamp": "2026-03-18T10:00:00Z"
  },
  "commits": [
    {
      "id": "abc123456789",
      "message": "Ship dashboard shell",
      "url": "https://github.com/acme/dmxforge/commit/abc123456789",
      "author": { "name": "Acme" }
    }
  ],
  "repository": {
    "name": "dmxforge",
    "full_name": "acme/dmxforge",
    "html_url": "https://github.com/acme/dmxforge"
  },
  "sender": {
    "login": "acme",
    "html_url": "https://github.com/acme",
    "avatar_url": "https://avatars.githubusercontent.com/u/1?v=4"
  }
}
```

### Exemple `pull_request`

```http
POST /webhooks/github/abc123
Content-Type: application/json
X-GitHub-Event: pull_request
X-Hub-Signature-256: sha256=<signature>
```

```json
{
  "action": "closed",
  "pull_request": {
    "number": 42,
    "merged": true,
    "title": "Refactor Discord rendering",
    "body": "Cleanup templates and preview rendering.",
    "html_url": "https://github.com/acme/dmxforge/pull/42",
    "head": { "ref": "feature/rendering" },
    "base": { "ref": "main" },
    "updated_at": "2026-03-18T11:00:00Z"
  },
  "repository": {
    "name": "dmxforge",
    "full_name": "acme/dmxforge",
    "html_url": "https://github.com/acme/dmxforge"
  },
  "sender": {
    "login": "acme"
  }
}
```

Evenements GitHub supportes :

- `push`
- `pull_request`
- `issues`
- `release`

## GitLab

### Headers attendus

| Header | Usage |
| --- | --- |
| `X-Gitlab-Event` | Type d'evenement |
| `X-Gitlab-Token` | Verification par comparaison directe si un secret est configure |

### Exemple `merge_request`

```http
POST /webhooks/gitlab/abc123
Content-Type: application/json
X-Gitlab-Event: Merge Request Hook
X-Gitlab-Token: <secret>
```

```json
{
  "object_kind": "merge_request",
  "user_name": "Acme",
  "user_username": "acme",
  "project": {
    "name": "dmxforge",
    "path_with_namespace": "acme/dmxforge",
    "web_url": "https://gitlab.example.com/acme/dmxforge"
  },
  "object_attributes": {
    "iid": 42,
    "title": "Refactor Discord rendering",
    "description": "Cleanup templates and preview rendering.",
    "source_branch": "feature/rendering",
    "target_branch": "main",
    "state": "merged",
    "url": "https://gitlab.example.com/acme/dmxforge/-/merge_requests/42",
    "updated_at": "2026-03-18T11:00:00Z"
  }
}
```

### Exemple `pipeline`

```http
POST /webhooks/gitlab/abc123
Content-Type: application/json
X-Gitlab-Event: Pipeline Hook
X-Gitlab-Token: <secret>
```

```json
{
  "object_kind": "pipeline",
  "user_name": "Acme",
  "user_username": "acme",
  "project": {
    "name": "dmxforge",
    "path_with_namespace": "acme/dmxforge",
    "web_url": "https://gitlab.example.com/acme/dmxforge"
  },
  "object_attributes": {
    "id": 512,
    "name": "CI main",
    "status": "success",
    "ref": "main",
    "url": "https://gitlab.example.com/acme/dmxforge/-/pipelines/512",
    "created_at": "2026-03-18T12:00:00Z"
  }
}
```

Evenements GitLab supportes :

- `push`
- `tag_push`
- `merge_request`
- `pipeline`
- `release`

## Gitea / Forgejo

### Headers attendus

| Header | Usage |
| --- | --- |
| `X-Gitea-Event-Type` | Type d'evenement |
| `X-Gitea-Signature` | Verification HMAC SHA-256 si un secret est configure |
| `X-Gogs-Signature` | Fallback compatible |

### Exemple `release`

```http
POST /webhooks/gitea/abc123
Content-Type: application/json
X-Gitea-Event-Type: release
X-Gitea-Signature: <signature-hex>
```

```json
{
  "action": "published",
  "repository": {
    "name": "dmxforge",
    "full_name": "acme/dmxforge",
    "html_url": "https://gitea.example.com/acme/dmxforge"
  },
  "sender": {
    "login": "acme",
    "full_name": "Acme",
    "html_url": "https://gitea.example.com/acme",
    "avatar_url": "https://gitea.example.com/avatars/acme"
  },
  "release": {
    "tag_name": "v1.0.0",
    "name": "v1.0.0",
    "body": "First stable release",
    "html_url": "https://gitea.example.com/acme/dmxforge/releases/tag/v1.0.0",
    "published_at": "2026-03-18T13:00:00Z"
  }
}
```

### Exemple `push`

```http
POST /webhooks/gitea/abc123
Content-Type: application/json
X-Gitea-Event-Type: push
X-Gitea-Signature: <signature-hex>
```

```json
{
  "ref": "refs/heads/main",
  "compare_url": "https://gitea.example.com/acme/dmxforge/compare/abc1234...def5678",
  "head_commit": {
    "message": "Ship dashboard shell",
    "timestamp": "2026-03-18T10:00:00Z"
  },
  "commits": [
    {
      "id": "abc123456789",
      "message": "Ship dashboard shell",
      "url": "https://gitea.example.com/acme/dmxforge/commit/abc123456789",
      "author": { "name": "Acme" }
    }
  ],
  "repository": {
    "name": "dmxforge",
    "full_name": "acme/dmxforge",
    "html_url": "https://gitea.example.com/acme/dmxforge"
  },
  "sender": {
    "login": "acme",
    "full_name": "Acme"
  }
}
```

Evenements Gitea supportes :

- `push`
- `pull_request`
- `issues`
- `release`

## Normalisation

Quel que soit le provider, DmxForge convertit ensuite le payload en structure commune exposee aux templates :

- `provider`
- `event_type`
- `repository.*`
- `actor.*`
- `branch`
- `compare_url`
- `commit_count`
- `commits[]`
- `title`
- `description`
- `status`
- `url`
- `timestamp`
- `metadata`

Le champ `metadata` conserve le payload brut provider-specifique pour les cas avances.
