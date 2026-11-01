# Documentation website

The Docusaurus source lives here. GitHub Pages deployment is managed by
[feuer-rs/feuer-rs.github.io](https://github.com/feuer-rs/feuer-rs.github.io).

Use Node.js 24:

```sh
cd website
npm ci
npm start
```

Run `npm run build` to check the production build and internal links.

To publish, run the **Deploy website** workflow in the website repository.
It checks out `PSeitz/feuer` at `main`, builds this directory, and deploys to
https://feuer-rs.github.io/. Source changes do not publish automatically.
