const path = require("path");
const CopyPlugin = require("copy-webpack-plugin");

const dist = path.resolve(__dirname, "dist");

module.exports = {
  mode: "production",
  entry: {
    index: "./src/main.js"
  },
  output: {
    path: dist,
    filename: "[name].js"
  },
  devServer: {
    static: dist,
    proxy: {
      "/api/": {
        target: 'https://lg.staging.service.wobcom.de'
      }
    },
  },
  plugins: [
    new CopyPlugin({
      patterns: [
        {
          from: "static",
          transform: (content) => {
            return content
              .toString()
              .replace('#VERSION#', process.env.FERNGLAS_VERSION)
              .replace('#COMMIT#', process.env.FERNGLAS_COMMIT);
          },
        }
      ]
    })
  ]
};

