import DefaultTheme from 'vitepress/theme'
import type { Theme } from 'vitepress'
import Screenshot from './components/Screenshot.vue'
import './custom.css'

export default {
  extends: DefaultTheme,
  enhanceApp({ app }) {
    app.component('Screenshot', Screenshot)
  }
} satisfies Theme
