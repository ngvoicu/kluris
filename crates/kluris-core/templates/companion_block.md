
## Spec-worthy work first

When the user proposes multi-step work (new feature, refactor across files, migration, new subsystem), pause before coding and follow the embedded companion playbook:

{routing}

Use the HTML companions when the user wants `.specs/<id>/SPEC.html` as the canonical spec document; otherwise prefer the markdown companions.

Each playbook is a folder: paths in it that start with `references/` or `assets/` resolve against the playbook's own directory (e.g. `{first_companion_dir}/`); `.specs/...` paths are project paths.

Kluris ships these playbooks embedded. Do not direct the user to install a separate specmint skill or package.

Small single-file edits do not need a spec; proceed directly.
