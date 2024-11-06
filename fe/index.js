  var socket = io();

  console.log("loaded script");
  document.getElementById('btn').onclick= () => {
	  socket.emit("message-with-ack", "yo", console.log)
  }
