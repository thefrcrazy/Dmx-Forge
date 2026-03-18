# Deploying with Traefik

Ce guide couvre le deploiement HTTPS avec `docker-compose.traefik.yml`.

## Prerequis

- Docker et Docker Compose
- un domaine public pointant vers le serveur
- ports `80` et `443` ouverts
- acces root ou equivalent pour lancer Docker

## Fichiers impliques

- `docker-compose.traefik.yml`
- `deploy/traefik.yml`
- `.env`

## Variables a definir

Exemple :

```dotenv
SECRET_KEY=replace-with-a-random-32-char-secret
SECURE_COOKIES=true
PUBLIC_DOMAIN=dmxforge.example.com
```

## Important : email ACME

Le fichier versionne `deploy/traefik.yml` contient encore un email placeholder :

```yaml
admin@example.com
```

Avant un vrai deploiement, remplacez-le par votre adresse.

## Lancement

```bash
docker compose -f docker-compose.traefik.yml up --build -d
```

Ce compose lance :

- `dmxforge`
- `traefik`

## Ce que fait Traefik ici

- expose `:80` et `:443`
- redirige HTTP vers HTTPS
- genere les certificats LetsEncrypt via challenge HTTP
- route `Host(${PUBLIC_DOMAIN})` vers le service DmxForge
- applique des middlewares headers et rate limit

## Volumes

| Volume | Role |
| --- | --- |
| `dmxforge-data` | persistance SQLite |
| `traefik-letsencrypt` | stockage des certificats ACME |

## Verifications apres boot

### 1. Conteneurs

```bash
docker compose -f docker-compose.traefik.yml ps
```

### 2. Healthcheck applicatif

```bash
curl https://dmxforge.example.com/health
```

Reponse attendue :

```json
{
  "status": "ok",
  "app_name": "DmxForge",
  "version": "0.1.0",
  "database": "sqlite",
  "timestamp": "2026-03-18T12:00:00Z"
}
```

### 3. Cookies

En HTTPS avec `SECURE_COOKIES=true`, verifiez dans le navigateur que le cookie de session est bien marque `Secure`.

## DNS

Le domaine de `PUBLIC_DOMAIN` doit resoudre vers l'IP publique du serveur.

Exemple :

```text
dmxforge.example.com -> A -> <server-ip>
```

## URLs webhook

L'UI de DmxForge derive les URLs webhook a partir des headers de requete forwards par Traefik. Il n'y a pas de `PUBLIC_BASE_URL` a definir.

Cela implique :

- utilisez bien le domaine public final dans Traefik
- laissez Traefik transmettre `Host` / `X-Forwarded-*`

## Mise a jour

```bash
docker compose -f docker-compose.traefik.yml up --build -d
```

Les donnees SQLite restent sur le volume `dmxforge-data`.

## Sauvegarde

Les elements critiques a sauvegarder sont :

- le volume `dmxforge-data`
- le volume `traefik-letsencrypt`

## Troubleshooting

### Pas de certificat

Verifier :

- que `PUBLIC_DOMAIN` resolv vers le serveur
- que le port `80` est bien accessible publiquement
- que l'email ACME a ete corrige dans `deploy/traefik.yml`

### Redirection ou URL publique incorrecte

Verifier :

- la regle `Host(...)` Traefik
- les en-tetes forwarded
- le domaine utilise par le navigateur

### Login fonctionne mal en prod

Verifier :

- `SECURE_COOKIES=true`
- HTTPS actif
- pas de mismatch de domaine entre la page chargee et le cookie

### Webhooks Discord ou URLs sources semblent locales

Verifier :

- que l'instance est bien atteinte via le domaine public
- que Traefik forwarde le bon `Host`

## Alternative Caddy

Le depot fournit aussi un exemple Caddy dans `deploy/Caddyfile.example`.

Ce n'est pas le chemin principal documente ici, mais il peut servir de base si vous preferez Caddy a Traefik.
