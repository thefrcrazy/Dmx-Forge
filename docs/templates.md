# Templates Discord Minijinja

DmxForge utilise Minijinja pour rendre le corps des messages Discord.

Le rendu de preview et le rendu runtime partagent la meme idee : un payload JSON est passe au moteur de template, puis la chaine produite est injectee dans le message Discord.

## Comportement du moteur

- moteur : Minijinja
- auto-escape : desactive
- entree : JSON normalise
- sortie attendue : texte ou markdown Discord

Consequence importante :

- n'ecrivez pas de HTML
- pensez a vos retours a la ligne
- utilisez des valeurs de secours pour les champs optionnels

## Variables disponibles

Le contexte commun expose les champs suivants.

| Variable | Type | Notes |
| --- | --- | --- |
| `provider` | string | `github`, `gitlab`, `gitea` |
| `event_type` | string | ex. `push`, `pull_request`, `merge_request`, `pipeline`, `release`, `issues` |
| `repository.name` | string | nom court du repo |
| `repository.full_name` | string | nom qualifie, ex. `owner/repo` |
| `repository.url` | string? | URL web du repository si disponible |
| `actor.name` | string | nom affiche pour l'auteur / emetteur |
| `actor.username` | string | login ou username |
| `actor.url` | string? | URL profil si disponible |
| `actor.avatar_url` | string? | URL avatar si disponible |
| `branch` | string? | branche ou ref principale de l'evenement |
| `compare_url` | string? | URL compare / diff si disponible |
| `commit_count` | number | nombre de commits normalises |
| `commits` | array | liste des commits |
| `title` | string? | titre de PR / release / issue / pipeline |
| `description` | string? | corps descriptif de l'evenement |
| `status` | string? | statut ou action normalisee |
| `url` | string? | URL principale de l'evenement |
| `timestamp` | string | timestamp RFC3339 |
| `metadata` | object | payload brut provider-specifique |

## Structure de `commits`

Chaque commit normalise peut contenir :

| Variable | Type | Notes |
| --- | --- | --- |
| `commit.id` | string | SHA complete ou identifiant brut |
| `commit.short_id` | string? | SHA courte si disponible |
| `commit.message` | string | message de commit |
| `commit.url` | string? | lien vers le commit |
| `commit.author_name` | string? | auteur si disponible |

## Disponibilite selon le type d'evenement

| Type | Champs les plus utiles |
| --- | --- |
| `push` | `branch`, `commit_count`, `commits`, `compare_url` |
| `tag_push` | `branch`, `commit_count`, `commits`, `compare_url` |
| `pull_request` | `title`, `description`, `status`, `url`, `metadata.pull_request` |
| `merge_request` | `title`, `description`, `status`, `url`, `metadata.object_attributes` |
| `pipeline` | `title`, `status`, `url`, `branch`, `metadata.object_attributes` |
| `release` | `title`, `description`, `status`, `url`, `branch`, `metadata.release` |
| `issues` | `title`, `description`, `status`, `url`, `metadata.issue` |

## Notes preview vs runtime

Le preview fourni par l'UI repose sur un sample payload embarque. Il couvre les champs les plus courants, mais pas tout le payload runtime.

Exemple :

- le sample preview contient `commits[].id` et `commits[].message`
- certains champs comme `commits[].short_id`, `commit.url` ou `actor.avatar_url` peuvent etre absents du sample mais presents a l'execution reelle

Conclusion :

- utilisez le preview pour la mise en forme
- utilisez `default` et des gardes conditionnels pour les champs optionnels

## Exemples de templates

### Push compact

```jinja
{{ actor.name }} pushed {{ commit_count }} commits to {{ repository.full_name }}
{% if branch %}on {{ branch }}{% endif %}

{% for commit in commits %}
- {{ commit.short_id or commit.id }} {{ commit.message }}
{% endfor %}
```

### PR / MR / release simple

```jinja
{% if title %}**{{ title }}**{% endif %}
{% if description %}
{{ description }}
{% endif %}

repo: {{ repository.full_name }}
{% if branch %}branch: {{ branch }}{% endif %}
{% if status %}status: {{ status }}{% endif %}
```

### Fallback defensif

```jinja
{{ actor.name | default("unknown") }} triggered {{ event_type | default("event") }}
for {{ repository.full_name | default("unknown/repo") }}
```

### Acces a `metadata`

GitHub pull request :

```jinja
PR #{{ metadata.pull_request.number }} -> {{ metadata.pull_request.base.ref }}
```

GitLab merge request :

```jinja
MR !{{ metadata.object_attributes.iid }} targeting {{ metadata.object_attributes.target_branch }}
```

Gitea release :

```jinja
Release tag: {{ metadata.release.tag_name }}
```

## Bonnes pratiques

- gardez les templates lisibles et courts
- evitez de supposer qu'un champ optionnel existe
- testez les templates via l'UI avant de les associer a une regle
- utilisez `metadata` seulement si le contexte normalise ne suffit pas
- evitez les blocs trop longs qui rendront mal dans Discord

## Preview API

Endpoint :

```text
POST /api/preview
```

Payload JSON :

```json
{
  "template": "{{ actor.name }} pushed {{ commit_count }} commits"
}
```

Reponse JSON :

```json
{
  "rendered": "Acme pushed 3 commits",
  "sample_payload": {
    "provider": "github",
    "event_type": "push"
  }
}
```

Note :

- cet endpoint est destine a l'UI authentifiee
- il verifie les permissions de lecture templates

## Limitations actuelles

- pas de coloration syntaxique Jinja dediee dans l'UI
- preview base sur un sample payload unique, pas sur tous les types d'evenement
- auto-escape desactive : le rendu est textuel, pas HTML
