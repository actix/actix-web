import Vue from 'vue'
import App from './app'

new Vue({
  el: '#app',
  render: h => h(App)
})

if (module.hot) {
  module.hot.accept();
}