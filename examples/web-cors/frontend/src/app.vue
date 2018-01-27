<template>
 <div id="app">
   <div id="content">
        <div id="title">    
            <a to="#">SignUp</a> 
        </div> 
          <input type="text" name="username" placeholder="Username" v-model="Username"  required />
          <input type="text" name="email" placeholder="E-mail" v-model="Email"  required />
          <input type="password" name="password" placeholder="Password" v-model="Password"  required/>
          <input type="password" name="confirm_password" placeholder="Confirm password" v-model="ConfirmPassword"  required/><br/>
          
          <button id="submit" @click="signup">Sign up</button>

          <div id="user-info">
              <p>Click Above 'Sign up' Button <br> Then Get Your Signup Info!</p>
              <p>email : {{ email }}</p>
              <p>username ：{{ username }}</p>
              <p>password : {{ password }}</p>
          </div>
    </div>
  </div>
</template>

<script>
import axios from 'axios'
export default {
  name: 'app',
  data () {
    return {
      Username: '',
      Email: '',
      Password: '',
      ConfirmPassword: '',

      email: '',
      username: '',
      password: ''
    }
  },
  methods: {
    signup () {
      var username = this.Username
      var email = this.Email
      var password = this.Password
      var confirm_password = this.ConfirmPassword
      console.log(email)
      axios.post('http://localhost:8000/user/info', {
          username: username,
          email: email,
          password: password,
          confirm_password: confirm_password
      })
      .then(response => {
            console.log(response.data)
            this.email =  response.data.email
            this.username =  response.data.username
            this.password =  response.data.password
      })
      .catch(e => {
        console.log(e)
      })
    }
  }
}
</script>

<style scoped>
#content {
  width: 250px;
  margin: 0 auto;
  padding-top: 33px;
}
#title {
    padding: 0.5rem 0;
    font-size: 22px;
    font-weight: bold;
    background-color:bisque;
    text-align: center;
}
input[type="text"],
input[type="password"] {
  margin: 6px auto auto;
  width: 250px;
  height: 36px;
  border: none;
  border-bottom: 1px solid #AAA;
  font-size: 16px;
}
#submit  {
  margin: 10px 0 20px 0;
  width: 250px;
  height: 33px;
  background-color:bisque;
  border: none;
  border-radius: 2px;
  font-family: 'Roboto', sans-serif;
  font-weight: bold;
  text-transform: uppercase;
  transition: 0.1s ease;
  cursor: pointer;
}
input[type="checkbox"] {
  margin-top: 11px;
}
dialog {
top: 50%;
width: 80%;  
border: 5px solid rgba(0, 0, 0, 0.3);
}
dialog::backdrop{
position: fixed;
top: 0;
left: 0;
right: 0;
bottom: 0;
background-color: rgba(0, 0, 0, 0.7);
}
#closeDialog {
display: inline-block;
border-radius: 3px;
border: none;
font-size: 1rem;
padding: 0.4rem 0.8em;
background: #eb9816;
border-bottom: 1px solid #f1b75c;
color: white;
font-weight: bold;
text-align: center;
}
#closeDialog:hover, #closeDialog:focus {
opacity: 0.92;
cursor: pointer;
}
#user-info {
  width: 250px;
  margin: 0 auto;
  padding-top: 44px;
}
@media only screen and (min-width: 600px) {
    #content  {
      margin: 0 auto;
      padding-top: 100px;
  }
}
</style>