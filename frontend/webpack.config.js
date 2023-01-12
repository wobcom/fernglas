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
      "/query": {
        target: 'http://localhost:3000'
      }
    },
  },
  plugins: [
    new CopyPlugin({
      patterns: [ "static" ]
    })
  ]
};

