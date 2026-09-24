import { createApp } from "vue";
import { createPinia } from "pinia";
import PopupApp from "./PopupApp.vue";
import "../assets/styles/main.css";

createApp(PopupApp).use(createPinia()).mount("#popup");
