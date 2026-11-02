// 应用入口：挂载 Vue 根组件，并全局引入样式表。
import { createApp } from 'vue';
import './style.css';
import App from './App.vue';

createApp(App).mount('#app');
