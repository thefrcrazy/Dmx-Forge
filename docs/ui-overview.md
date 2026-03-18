# UI Overview

Le depot ne versionne pas encore de fichiers screenshot. Ce document decrit donc l'interface ecran par ecran pour completer le README.

## Dashboard

Route : `/dashboard`

Contenu principal :

- cartes de metriques globales
- activite sur 7 jours
- top repositories
- top event types
- dernieres livraisons

Objectif :

- verifier rapidement que les webhooks entrent
- voir ce qui est traite ou en erreur
- reperer les repos et evenements dominants

## Sources

Route : `/sources`

Fonctions :

- lister les sources webhook configurees
- creer une source GitHub / GitLab / Gitea
- afficher l'URL webhook complete a copier
- definir les filtres depots / branches / evenements
- activer / desactiver / supprimer

## Destinations

Route : `/destinations`

Fonctions :

- stocker les webhooks Discord de sortie
- masquer visuellement l'URL
- activer / desactiver / supprimer

## Templates

Route : `/templates`

Fonctions :

- gerer les templates Minijinja
- configurer style, couleur, footer, username, avatar
- activer / desactiver
- previsualiser le rendu texte et le faux embed Discord

## Routes

Route : `/rules`

Fonctions :

- associer source, template et destination
- ajouter des filtres optionnels
- tester une route via l'UI
- voir une preview de flux et de rendu Discord

## Deliveries

Route : `/deliveries`

Fonctions :

- filtrer l'historique
- consulter les erreurs
- ouvrir le detail d'une livraison
- rejouer une livraison si le role le permet

## Users

Route : `/users`

Fonctions :

- lister les utilisateurs visibles dans le scope courant
- creer des sous-utilisateurs
- modifier role, statut et permissions deleguees
- desactiver ou supprimer un compte selon les regles de securite

## Login et setup

Routes :

- `/login`
- `/setup`

Comportement :

- si aucun admin n'existe, l'app force `/setup`
- sinon l'app force l'auth avant l'acces aux pages admin

## Navigation

Le shell affiche conditionnellement les entrees de navigation selon les permissions du compte courant.

Exemples :

- un `viewer` verra les pages en lecture seulement
- un utilisateur sans droit `templates_read` ne verra pas l'entree Templates
- un utilisateur sans droit `users_read` ne verra pas la page Utilisateurs
